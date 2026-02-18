use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::cache::Cache;
use crate::config::Config;
use crate::feed::{Article, FeedMeta};

pub enum BackendCommand {
    FetchAllFeeds,
    FetchFeed { url: String },
    MarkRead { hash: String, read: bool },
    MarkFeedRead { feed_url: String },
    Shutdown,
}

pub enum BackendResponse {
    FeedArticles {
        feed_url: String,
        feed_name: String,
        fetched_at: String,
        articles: Vec<Article>,
        total: usize,
        unread: usize,
    },
    FetchError {
        feed_url: String,
        error: String,
    },
    ArticleMutation {
        hash: String,
        read: bool,
    },
    FeedMarkedRead {
        feed_url: String,
    },
}

pub fn spawn(
    config: &Config,
) -> (
    mpsc::Sender<BackendCommand>,
    mpsc::Receiver<BackendResponse>,
) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<BackendCommand>();
    let (resp_tx, resp_rx) = mpsc::channel::<BackendResponse>();

    let feeds = config.feeds.clone();
    let sync_interval = Duration::from_secs(config.ui.sync_interval_secs);

    std::thread::spawn(move || {
        let cache = match Cache::open() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to open cache: {}", e);
                return;
            }
        };
        backend_loop(cmd_rx, resp_tx, &cache, &feeds, sync_interval);
    });

    (cmd_tx, resp_rx)
}

fn backend_loop(
    cmd_rx: mpsc::Receiver<BackendCommand>,
    resp_tx: mpsc::Sender<BackendResponse>,
    cache: &Cache,
    feeds: &[crate::config::FeedConfig],
    sync_interval: Duration,
) {
    let mut last_fetch = Instant::now();

    loop {
        let timeout = sync_interval.saturating_sub(last_fetch.elapsed());
        match cmd_rx.recv_timeout(timeout) {
            Ok(cmd) => match cmd {
                BackendCommand::FetchAllFeeds => {
                    for feed in feeds {
                        fetch_one_feed(&feed.url, &feed.name, cache, &resp_tx);
                    }
                    last_fetch = Instant::now();
                }
                BackendCommand::FetchFeed { url } => {
                    let name = feeds
                        .iter()
                        .find(|f| f.url == url)
                        .map(|f| f.name.as_str())
                        .unwrap_or("Unknown");
                    fetch_one_feed(&url, name, cache, &resp_tx);
                }
                BackendCommand::MarkRead { hash, read } => {
                    cache.mark_read(&hash, read);
                    let _ = resp_tx.send(BackendResponse::ArticleMutation { hash, read });
                }
                BackendCommand::MarkFeedRead { feed_url } => {
                    if let Some(hashes) = cache.get_feed_index(&feed_url) {
                        for h in &hashes {
                            cache.mark_read(h, true);
                        }
                    }
                    let _ = resp_tx.send(BackendResponse::FeedMarkedRead { feed_url });
                }
                BackendCommand::Shutdown => break,
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Periodic refresh: re-fetch all feeds
                for feed in feeds {
                    fetch_one_feed(&feed.url, &feed.name, cache, &resp_tx);
                }
                last_fetch = Instant::now();
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn fetch_one_feed(url: &str, name: &str, cache: &Cache, resp_tx: &mpsc::Sender<BackendResponse>) {
    match crate::feed::fetch_feed(url, name) {
        Ok((title, articles)) => {
            let fetched_at = now_local_timestamp();
            cache.put_feed_meta(&FeedMeta {
                url: url.to_string(),
                title,
                last_fetched: fetched_at.clone(),
            });

            // Deduplicate against cache
            let new_articles: Vec<_> = articles
                .into_iter()
                .filter(|a| !cache.article_exists(&a.hash))
                .collect();

            // Store in cache
            if !new_articles.is_empty() {
                cache.put_articles(&new_articles);
            }

            // Update feed index
            let mut hashes = cache.get_feed_index(url).unwrap_or_default();
            for a in &new_articles {
                if !hashes.contains(&a.hash) {
                    hashes.insert(0, a.hash.clone());
                }
            }
            cache.put_feed_index(url, &hashes);

            let unread = hashes
                .iter()
                .filter(|h| cache.get_article(h).map(|a| !a.read).unwrap_or(false))
                .count();

            let _ = resp_tx.send(BackendResponse::FeedArticles {
                feed_url: url.to_string(),
                feed_name: name.to_string(),
                fetched_at,
                articles: new_articles,
                total: hashes.len(),
                unread,
            });
        }
        Err(e) => {
            let _ = resp_tx.send(BackendResponse::FetchError {
                feed_url: url.to_string(),
                error: e,
            });
        }
    }
}

fn now_local_timestamp() -> String {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm = std::mem::zeroed::<libc::tm>();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return "unknown".to_string();
        }
        let mut buf = [0u8; 32];
        let fmt = b"%Y-%m-%d %H:%M:%S\0";
        let n = libc::strftime(
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
            fmt.as_ptr() as *const libc::c_char,
            &tm,
        );
        if n == 0 {
            "unknown".to_string()
        } else {
            String::from_utf8_lossy(&buf[..n as usize]).to_string()
        }
    }
}
