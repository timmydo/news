use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::cache::Cache;
use crate::config::Config;
use crate::feed::{now_local_datetime_string, Article, FeedMeta};
use crate::log;

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
                log::error(format!("backend failed to open cache: {}", e));
                eprintln!("Failed to open cache: {}", e);
                return;
            }
        };
        log::info("backend thread started");
        backend_loop(cmd_rx, resp_tx, &cache, &feeds, sync_interval);
        log::info("backend thread stopped");
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
                    log::info("backend command: FetchAllFeeds");
                    for feed in feeds {
                        fetch_one_feed(&feed.url, &feed.name, cache, &resp_tx);
                    }
                    last_fetch = Instant::now();
                }
                BackendCommand::FetchFeed { url } => {
                    log::info(format!("backend command: FetchFeed {}", url));
                    let name = feeds
                        .iter()
                        .find(|f| f.url == url)
                        .map(|f| f.name.as_str())
                        .unwrap_or("Unknown");
                    fetch_one_feed(&url, name, cache, &resp_tx);
                }
                BackendCommand::MarkRead { hash, read } => {
                    log::info(format!("backend command: MarkRead {} => {}", hash, read));
                    cache.mark_read(&hash, read);
                    let _ = resp_tx.send(BackendResponse::ArticleMutation { hash, read });
                }
                BackendCommand::MarkFeedRead { feed_url } => {
                    log::info(format!("backend command: MarkFeedRead {}", feed_url));
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
                log::info("backend periodic refresh");
                for feed in feeds {
                    fetch_one_feed(&feed.url, &feed.name, cache, &resp_tx);
                }
                last_fetch = Instant::now();
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                log::warn("backend command channel disconnected");
                break;
            }
        }
    }
}

fn fetch_one_feed(url: &str, name: &str, cache: &Cache, resp_tx: &mpsc::Sender<BackendResponse>) {
    log::info(format!("fetch start: {} ({})", name, url));
    match crate::feed::fetch_feed(url, name) {
        Ok((title, articles)) => {
            let fetched_at = now_local_datetime_string();
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

            log::info(format!(
                "fetch success: {} ({}) new={} total={} unread={}",
                name,
                url,
                new_articles.len(),
                hashes.len(),
                unread
            ));

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
            log::error(format!("fetch error: {} ({}) {}", name, url, e));
            let _ = resp_tx.send(BackendResponse::FetchError {
                feed_url: url.to_string(),
                error: e,
            });
        }
    }
}
