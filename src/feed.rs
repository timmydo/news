use serde::{Deserialize, Serialize};

/// A normalized article parsed from an RSS or Atom feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Article {
    pub hash: String,
    pub title: String,
    pub link: String,
    pub description: String,
    pub content: String,
    pub published: Option<String>,
    pub feed_name: String,
    pub read: bool,
}

/// Metadata about a feed after fetching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedMeta {
    pub url: String,
    pub title: String,
    pub last_fetched: String,
}

/// Fetch and parse an RSS/Atom feed from the given URL.
/// Returns the feed title and a list of articles.
pub fn fetch_feed(url: &str, feed_name: &str) -> Result<(String, Vec<Article>), String> {
    let resp = ureq::get(url)
        .timeout(std::time::Duration::from_secs(30))
        .call()
        .map_err(|e| format!("fetch {}: {}", url, e))?;

    let body = resp
        .into_string()
        .map_err(|e| format!("read body from {}: {}", url, e))?;

    parse_feed(&body, feed_name)
}

/// Parse RSS/Atom XML into articles.
fn parse_feed(xml: &str, feed_name: &str) -> Result<(String, Vec<Article>), String> {
    // TODO: implement XML parsing for RSS 2.0 and Atom feeds
    // For now, return an error indicating this is not yet implemented
    let _ = (xml, feed_name);
    Err("feed parsing not yet implemented".to_string())
}

/// Compute a deduplication hash for an article (matching feed2maildir's approach).
pub fn article_hash(title: &str, link: &str) -> String {
    // Simple hash based on title + link
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    title.hash(&mut hasher);
    link.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
