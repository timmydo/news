use redb::{Database, ReadableTable, TableDefinition};
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

    pub fn list_feed_urls(&self) -> Vec<String> {
        let mut urls = Vec::<String>::new();
        let txn = match self.db.begin_read() {
            Ok(txn) => txn,
            Err(_) => return urls,
        };

        if let Ok(table) = txn.open_table(FEED_INDEX) {
            if let Ok(iter) = table.iter() {
                for entry in iter {
                    if let Ok((key, _)) = entry {
                        urls.push(key.value().to_string());
                    }
                }
            }
        }
        if let Ok(table) = txn.open_table(FEEDS) {
            if let Ok(iter) = table.iter() {
                for entry in iter {
                    if let Ok((key, _)) = entry {
                        urls.push(key.value().to_string());
                    }
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
