use redb::{Database, ReadableTable, TableDefinition};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use crate::feed::{Article, FeedMeta};

const ARTICLES: TableDefinition<&str, &str> = TableDefinition::new("articles");
const FEEDS: TableDefinition<&str, &str> = TableDefinition::new("feeds");
const FEED_INDEX: TableDefinition<&str, &str> = TableDefinition::new("feed_index");

pub struct Cache {
    db: Arc<Database>,
}

pub struct FeedRefresh {
    pub url: String,
    pub name: String,
    pub title: String,
    pub fetched_at: String,
    pub articles: Vec<Article>,
}

pub struct FeedRefreshSummary {
    pub feed_url: String,
    pub feed_name: String,
    pub fetched_at: String,
    pub new_articles: usize,
    pub total: usize,
    pub unread: usize,
}

impl Cache {
    pub fn open() -> Result<Cache, String> {
        let path = Self::default_db_path();
        Self::open_at(path)
    }

    pub fn open_at(path: PathBuf) -> Result<Cache, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create cache dir: {}", e))?;
        }
        let db =
            Database::create(&path).map_err(|e| format!("open cache {}: {}", path.display(), e))?;
        Ok(Cache { db: Arc::new(db) })
    }

    pub fn default_db_path() -> PathBuf {
        let xdg = std::env::var("XDG_CACHE_HOME").unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            format!("{}/.cache", home)
        });
        PathBuf::from(xdg).join("tn").join("tn.redb")
    }

    pub fn clear() {
        let path = Self::default_db_path();
        Self::clear_at(path);
    }

    pub fn clear_at(path: PathBuf) {
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }

    pub fn get_article(&self, hash: &str) -> Option<Article> {
        let txn = self.db.begin_read().ok()?;
        let table = txn.open_table(ARTICLES).ok()?;
        let val = table.get(hash).ok()??;
        serde_json::from_str(val.value()).ok()
    }

    pub fn put_articles(&self, articles: &[Article]) {
        let txn = self.db.begin_write().unwrap();
        {
            let mut table = txn.open_table(ARTICLES).unwrap();
            for article in articles {
                let json = serde_json::to_string(article).unwrap();
                table.insert(article.hash.as_str(), json.as_str()).unwrap();
            }
        }
        txn.commit().unwrap();
    }

    pub fn get_feed_meta(&self, url: &str) -> Option<FeedMeta> {
        let txn = self.db.begin_read().ok()?;
        let table = txn.open_table(FEEDS).ok()?;
        let val = table.get(url).ok()??;
        serde_json::from_str(val.value()).ok()
    }

    #[cfg(test)]
    pub fn put_feed_meta(&self, meta: &FeedMeta) {
        let txn = self.db.begin_write().unwrap();
        {
            let mut table = txn.open_table(FEEDS).unwrap();
            let json = serde_json::to_string(meta).unwrap();
            table.insert(meta.url.as_str(), json.as_str()).unwrap();
        }
        txn.commit().unwrap();
    }

    pub fn get_feed_index(&self, feed_url: &str) -> Option<Vec<String>> {
        let txn = self.db.begin_read().ok()?;
        let table = txn.open_table(FEED_INDEX).ok()?;
        let val = table.get(feed_url).ok()??;
        serde_json::from_str(val.value()).ok()
    }

    #[cfg(test)]
    pub fn put_feed_index(&self, feed_url: &str, hashes: &[String]) {
        let txn = self.db.begin_write().unwrap();
        {
            let mut table = txn.open_table(FEED_INDEX).unwrap();
            let json = serde_json::to_string(hashes).unwrap();
            table.insert(feed_url, json.as_str()).unwrap();
        }
        txn.commit().unwrap();
    }

    pub fn apply_refresh_batch(&self, refreshes: &[FeedRefresh]) -> Vec<FeedRefreshSummary> {
        if refreshes.is_empty() {
            return Vec::new();
        }

        let txn = self.db.begin_write().unwrap();
        let mut new_hashes_by_url = HashMap::<String, Vec<String>>::new();

        {
            let mut articles_table = txn.open_table(ARTICLES).unwrap();
            let mut known_new_hashes = HashSet::<String>::new();
            for refresh in refreshes {
                for article in &refresh.articles {
                    let exists = articles_table.get(article.hash.as_str()).unwrap().is_some()
                        || known_new_hashes.contains(&article.hash);
                    if !exists {
                        let json = serde_json::to_string(article).unwrap();
                        articles_table
                            .insert(article.hash.as_str(), json.as_str())
                            .unwrap();
                        known_new_hashes.insert(article.hash.clone());
                        new_hashes_by_url
                            .entry(refresh.url.clone())
                            .or_default()
                            .push(article.hash.clone());
                    }
                }
            }
        }

        {
            let mut feeds_table = txn.open_table(FEEDS).unwrap();
            for refresh in refreshes {
                let meta = FeedMeta {
                    url: refresh.url.clone(),
                    title: refresh.title.clone(),
                    last_fetched: refresh.fetched_at.clone(),
                };
                let json = serde_json::to_string(&meta).unwrap();
                feeds_table
                    .insert(meta.url.as_str(), json.as_str())
                    .unwrap();
            }
        }

        let mut hashes_by_url = HashMap::<String, Vec<String>>::new();
        {
            let mut index_table = txn.open_table(FEED_INDEX).unwrap();
            for refresh in refreshes {
                let mut hashes: Vec<String> = index_table
                    .get(refresh.url.as_str())
                    .unwrap()
                    .and_then(|val| serde_json::from_str(val.value()).ok())
                    .unwrap_or_default();

                if let Some(new_hashes) = new_hashes_by_url.get(&refresh.url) {
                    for hash in new_hashes {
                        if !hashes.contains(hash) {
                            hashes.insert(0, hash.clone());
                        }
                    }
                }

                let json = serde_json::to_string(&hashes).unwrap();
                index_table
                    .insert(refresh.url.as_str(), json.as_str())
                    .unwrap();
                hashes_by_url.insert(refresh.url.clone(), hashes);
            }
        }

        let mut summaries = Vec::with_capacity(refreshes.len());
        {
            let articles_table = txn.open_table(ARTICLES).unwrap();
            for refresh in refreshes {
                let hashes = hashes_by_url.get(&refresh.url).cloned().unwrap_or_default();
                let unread = hashes
                    .iter()
                    .filter(|hash| {
                        articles_table
                            .get(hash.as_str())
                            .unwrap()
                            .and_then(|val| serde_json::from_str::<Article>(val.value()).ok())
                            .map(|article| !article.read)
                            .unwrap_or(false)
                    })
                    .count();
                summaries.push(FeedRefreshSummary {
                    feed_url: refresh.url.clone(),
                    feed_name: refresh.name.clone(),
                    fetched_at: refresh.fetched_at.clone(),
                    new_articles: new_hashes_by_url
                        .get(&refresh.url)
                        .map(|hashes| hashes.len())
                        .unwrap_or(0),
                    total: hashes.len(),
                    unread,
                });
            }
        }

        txn.commit().unwrap();
        summaries
    }

    pub fn mark_read(&self, hash: &str, read: bool) {
        if let Some(mut article) = self.get_article(hash) {
            article.read = read;
            self.put_articles(&[article]);
        }
    }

    pub fn list_feed_urls(&self) -> Vec<String> {
        let mut urls = Vec::<String>::new();
        let txn = match self.db.begin_read() {
            Ok(txn) => txn,
            Err(_) => return urls,
        };

        if let Ok(table) = txn.open_table(FEED_INDEX) {
            if let Ok(iter) = table.iter() {
                for (key, _) in iter.flatten() {
                    urls.push(key.value().to_string());
                }
            }
        }
        if let Ok(table) = txn.open_table(FEEDS) {
            if let Ok(iter) = table.iter() {
                for (key, _) in iter.flatten() {
                    urls.push(key.value().to_string());
                }
            }
        }

        urls.sort();
        urls.dedup();
        urls
    }

    pub fn list_feed_articles(&self, feed_url: &str) -> Vec<Article> {
        self.get_feed_index(feed_url)
            .unwrap_or_default()
            .iter()
            .filter_map(|hash| self.get_article(hash))
            .collect()
    }

    pub fn list_all_articles(&self) -> Vec<Article> {
        let mut articles = Vec::new();
        for url in self.list_feed_urls() {
            articles.extend(self.list_feed_articles(&url));
        }
        articles
    }

    pub fn list_unread_articles(&self) -> Vec<Article> {
        self.list_all_articles()
            .into_iter()
            .filter(|a| !a.read)
            .collect()
    }
}

impl Clone for Cache {
    fn clone(&self) -> Self {
        Cache {
            db: Arc::clone(&self.db),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const FEED_URL: &str = "https://example.com/feed";
    const FEED_NAME: &str = "Example Feed";

    #[test]
    fn refresh_batch_inserts_articles_and_summarizes_feed() {
        let dir = tempdir().expect("tempdir");
        let cache = Cache::open_at(dir.path().join("test.redb")).expect("cache");
        let summaries = cache.apply_refresh_batch(&[FeedRefresh {
            url: FEED_URL.to_string(),
            name: FEED_NAME.to_string(),
            title: FEED_NAME.to_string(),
            fetched_at: "2026-04-30 10:00:00".to_string(),
            articles: vec![article("a1", false)],
        }]);

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].new_articles, 1);
        assert_eq!(summaries[0].total, 1);
        assert_eq!(summaries[0].unread, 1);
        assert!(cache.get_article("a1").is_some());
        assert_eq!(cache.get_feed_index(FEED_URL).unwrap(), vec!["a1"]);
    }

    #[test]
    fn refresh_batch_preserves_existing_read_state() {
        let dir = tempdir().expect("tempdir");
        let cache = Cache::open_at(dir.path().join("test.redb")).expect("cache");
        cache.put_articles(&[article("a1", true)]);
        cache.put_feed_index(FEED_URL, &["a1".to_string()]);

        let summaries = cache.apply_refresh_batch(&[FeedRefresh {
            url: FEED_URL.to_string(),
            name: FEED_NAME.to_string(),
            title: FEED_NAME.to_string(),
            fetched_at: "2026-04-30 10:00:00".to_string(),
            articles: vec![article("a1", false)],
        }]);

        assert_eq!(summaries[0].new_articles, 0);
        assert_eq!(summaries[0].unread, 0);
        assert!(cache.get_article("a1").unwrap().read);
    }

    fn article(hash: &str, read: bool) -> Article {
        Article {
            hash: hash.to_string(),
            title: format!("Article {}", hash),
            link: format!("https://example.com/{}", hash),
            description: "desc".to_string(),
            content: "content".to_string(),
            published: Some("2026-04-30 09:00:00".to_string()),
            feed_name: FEED_NAME.to_string(),
            read,
        }
    }
}
