use std::sync::mpsc;
use std::time::Duration;

use crate::cache::{Cache, FeedRefresh, FeedRefreshSummary};
use crate::config::Config;
use crate::feed::{datetime_sort_key, now_local_datetime_string};
use crate::log;
use chrono::Local;

pub enum BackendCommand {
    FetchAllFeeds,
    FetchFeed { url: String },
    MarkRead { hash: String, read: bool },
    MarkFeedRead { feed_url: String },
    Shutdown,
}

pub enum BackendResponse {
    RefreshCompleted { reports: Vec<FeedRefreshReport> },
    ArticleMutation { hash: String, read: bool },
    FeedMarkedRead { feed_url: String },
}

pub struct FeedRefreshReport {
    pub feed_url: String,
    pub feed_name: String,
    pub fetched_at: Option<String>,
    pub new_articles: usize,
    pub total: usize,
    pub unread: usize,
    pub error: Option<String>,
}

pub fn spawn(
    config: &Config,
    cache: Cache,
) -> (
    mpsc::Sender<BackendCommand>,
    mpsc::Receiver<BackendResponse>,
) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<BackendCommand>();
    let (resp_tx, resp_rx) = mpsc::channel::<BackendResponse>();

    let feeds = config.feeds.clone();
    let sync_interval = Duration::from_secs(config.ui.sync_interval_secs);

    std::thread::spawn(move || {
        log::info("backend thread started");
        log::news("backend thread started");
        backend_loop(cmd_rx, resp_tx, &cache, &feeds, sync_interval);
        log::info("backend thread stopped");
        log::news("backend thread stopped");
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
    loop {
        let timeout = next_refresh_timeout(cache, feeds, sync_interval);
        match cmd_rx.recv_timeout(timeout) {
            Ok(cmd) => match cmd {
                BackendCommand::FetchAllFeeds => {
                    log::info("backend command: FetchAllFeeds");
                    log::news("manual refresh: all feeds");
                    let reports = refresh_feeds(feeds.iter(), cache);
                    let _ = resp_tx.send(BackendResponse::RefreshCompleted { reports });
                }
                BackendCommand::FetchFeed { url } => {
                    log::info(format!("backend command: FetchFeed {}", url));
                    let reports = refresh_feeds(feeds.iter().filter(|feed| feed.url == url), cache);
                    let _ = resp_tx.send(BackendResponse::RefreshCompleted { reports });
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
                // Periodic refresh: only fetch feeds whose last refresh is stale.
                let due = due_feed_indices(cache, feeds, sync_interval);
                if due.is_empty() {
                    continue;
                }
                log::info(format!("backend periodic refresh: {} due", due.len()));
                log::news(format!("scheduled refresh: {} due feed(s)", due.len()));
                let reports = refresh_feeds(due.into_iter().map(|idx| &feeds[idx]), cache);
                let _ = resp_tx.send(BackendResponse::RefreshCompleted { reports });
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                log::warn("backend command channel disconnected");
                break;
            }
        }
    }
}

fn due_feed_indices(
    cache: &Cache,
    feeds: &[crate::config::FeedConfig],
    sync_interval: Duration,
) -> Vec<usize> {
    let now = Local::now().timestamp();
    let interval_secs = sync_interval.as_secs() as i64;

    feeds
        .iter()
        .enumerate()
        .filter_map(|(idx, feed)| {
            let last_fetched = cache
                .get_feed_meta(&feed.url)
                .and_then(|meta| datetime_sort_key(Some(&meta.last_fetched)));

            let is_due = match last_fetched {
                Some(ts) => now.saturating_sub(ts) >= interval_secs,
                None => true,
            };

            if is_due {
                Some(idx)
            } else {
                None
            }
        })
        .collect()
}

fn next_refresh_timeout(
    cache: &Cache,
    feeds: &[crate::config::FeedConfig],
    sync_interval: Duration,
) -> Duration {
    if feeds.is_empty() {
        return sync_interval;
    }

    let now = Local::now().timestamp();
    let interval_secs = sync_interval.as_secs() as i64;
    let mut min_remaining = interval_secs;

    for feed in feeds {
        let last_fetched = cache
            .get_feed_meta(&feed.url)
            .and_then(|meta| datetime_sort_key(Some(&meta.last_fetched)));

        match last_fetched {
            Some(ts) => {
                let elapsed = now.saturating_sub(ts);
                if elapsed >= interval_secs {
                    return Duration::from_secs(0);
                }
                min_remaining = min_remaining.min(interval_secs - elapsed);
            }
            None => return Duration::from_secs(0),
        }
    }

    Duration::from_secs(min_remaining as u64)
}

fn refresh_feeds<'a, I>(feeds: I, cache: &Cache) -> Vec<FeedRefreshReport>
where
    I: IntoIterator<Item = &'a crate::config::FeedConfig>,
{
    let mut refreshes = Vec::new();
    let mut report_order = Vec::new();

    for feed in feeds {
        match fetch_one_feed(&feed.url, &feed.name) {
            Ok(refresh) => {
                report_order.push(Ok(refresh.url.clone()));
                refreshes.push(refresh);
            }
            Err(error) => {
                report_order.push(Err(FeedRefreshReport {
                    feed_url: feed.url.clone(),
                    feed_name: feed.name.clone(),
                    fetched_at: None,
                    new_articles: 0,
                    total: 0,
                    unread: 0,
                    error: Some(error),
                }));
            }
        }
    }

    let mut summaries_by_url = cache
        .apply_refresh_batch(&refreshes)
        .into_iter()
        .map(|summary| (summary.feed_url.clone(), summary))
        .collect::<std::collections::HashMap<_, _>>();

    report_order
        .into_iter()
        .filter_map(|item| match item {
            Ok(url) => summaries_by_url.remove(&url).map(report_from_summary),
            Err(report) => Some(report),
        })
        .collect()
}

fn report_from_summary(summary: FeedRefreshSummary) -> FeedRefreshReport {
    log::info(format!(
        "fetch success: {} ({}) new={} total={} unread={}",
        summary.feed_name, summary.feed_url, summary.new_articles, summary.total, summary.unread
    ));
    log::news(format!(
        "refresh done: {} ({}) new={} total={} unread={}",
        summary.feed_name, summary.feed_url, summary.new_articles, summary.total, summary.unread
    ));

    FeedRefreshReport {
        feed_url: summary.feed_url,
        feed_name: summary.feed_name,
        fetched_at: Some(summary.fetched_at),
        new_articles: summary.new_articles,
        total: summary.total,
        unread: summary.unread,
        error: None,
    }
}

fn fetch_one_feed(url: &str, name: &str) -> Result<FeedRefresh, String> {
    log::info(format!("fetch start: {} ({})", name, url));
    log::news(format!("refresh start: {} ({})", name, url));
    match crate::feed::fetch_feed(url, name) {
        Ok((title, articles)) => Ok(FeedRefresh {
            url: url.to_string(),
            name: name.to_string(),
            title,
            fetched_at: now_local_datetime_string(),
            articles,
        }),
        Err(e) => {
            log::error(format!("fetch error: {} ({}) {}", name, url, e));
            log::news(format!("refresh error: {} ({}) {}", name, url, e));
            Err(e)
        }
    }
}
