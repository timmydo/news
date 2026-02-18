use redb::{Database, TableDefinition};
use std::path::PathBuf;
use std::sync::Arc;

use crate::feed::{Article, FeedMeta};

const ARTICLES: TableDefinition<&str, &str> = TableDefinition::new("articles");
const FEEDS: TableDefinition<&str, &str> = TableDefinition::new("feeds");
const FEED_INDEX: TableDefinition<&str, &str> = TableDefinition::new("feed_index");

pub struct Cache {
    db: Arc<Database>,
}

impl Cache {
    pub fn open() -> Result<Cache, String> {
        let path = Self::db_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create cache dir: {}", e))?;
        }
        let db =
            Database::create(&path).map_err(|e| format!("open cache {}: {}", path.display(), e))?;
        Ok(Cache { db: Arc::new(db) })
    }

    fn db_path() -> PathBuf {
        let xdg = std::env::var("XDG_CACHE_HOME").unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            format!("{}/.cache", home)
        });
        PathBuf::from(xdg).join("tn").join("tn.redb")
    }

    pub fn clear() {
        let path = Self::db_path();
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

    pub fn put_feed_index(&self, feed_url: &str, hashes: &[String]) {
        let txn = self.db.begin_write().unwrap();
        {
            let mut table = txn.open_table(FEED_INDEX).unwrap();
            let json = serde_json::to_string(hashes).unwrap();
            table.insert(feed_url, json.as_str()).unwrap();
        }
        txn.commit().unwrap();
    }

    pub fn mark_read(&self, hash: &str, read: bool) {
        if let Some(mut article) = self.get_article(hash) {
            article.read = read;
            self.put_articles(&[article]);
        }
    }

    pub fn article_exists(&self, hash: &str) -> bool {
        self.get_article(hash).is_some()
    }
}

impl Clone for Cache {
    fn clone(&self) -> Self {
        Cache {
            db: Arc::clone(&self.db),
        }
    }
}
