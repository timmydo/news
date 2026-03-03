use serde::Deserialize;
use serde_json::json;
use std::cmp::Ordering;
use std::io::{self, BufRead, Write};

use crate::cache::Cache;
use crate::feed::{datetime_sort_key, Article};

const MAX_LIMIT: usize = 1000;

fn default_search_folder() -> String {
    "all".to_string()
}

#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Command {
    ListFolders,
    ListArticles {
        folder: String,
        #[serde(default)]
        offset: usize,
        #[serde(default)]
        limit: Option<usize>,
    },
    GetArticle {
        hash: String,
    },
    MarkRead {
        hash: String,
        read: bool,
    },
    MarkFolderRead {
        folder: String,
    },
    SearchArticles {
        query: String,
        #[serde(default = "default_search_folder")]
        folder: String,
        #[serde(default)]
        offset: usize,
        #[serde(default)]
        limit: Option<usize>,
    },
    Help,
    Quit,
}

pub fn run(cache: &Cache) -> Result<(), String> {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();

    for line_res in stdin.lock().lines() {
        let line = match line_res {
            Ok(line) => line,
            Err(e) => return Err(format!("failed to read stdin: {}", e)),
        };

        if line.trim().is_empty() {
            continue;
        }

        let command: Command = match serde_json::from_str(&line) {
            Ok(cmd) => cmd,
            Err(e) => {
                write_json_line(
                    &mut stdout,
                    &json!({
                        "ok": false,
                        "error": format!("invalid command JSON: {}", e),
                    }),
                )?;
                continue;
            }
        };

        let response = handle_command(cache, command);
        write_json_line(&mut stdout, &response)?;

        if response.get("quit").and_then(|v| v.as_bool()) == Some(true) {
            break;
        }
    }

    Ok(())
}

fn write_json_line(stdout: &mut dyn Write, value: &serde_json::Value) -> Result<(), String> {
    let line = serde_json::to_string(value)
        .map_err(|e| format!("failed to encode JSON response: {}", e))?;
    writeln!(stdout, "{}", line).map_err(|e| format!("failed to write stdout: {}", e))?;
    stdout
        .flush()
        .map_err(|e| format!("failed to flush stdout: {}", e))
}

fn handle_command(cache: &Cache, command: Command) -> serde_json::Value {
    match command {
        Command::ListFolders => {
            let mut folders = Vec::new();
            let all = sort_articles(cache.list_all_articles());
            let unread_count = all.iter().filter(|a| !a.read).count();

            folders.push(json!({
                "id": "all",
                "name": "All",
                "total": all.len(),
                "unread": unread_count,
                "virtual": true,
            }));
            folders.push(json!({
                "id": "unread",
                "name": "Unread",
                "total": unread_count,
                "unread": unread_count,
                "virtual": true,
            }));

            for url in cache.list_feed_urls() {
                let articles = cache.list_feed_articles(&url);
                let total = articles.len();
                let unread = articles.iter().filter(|a| !a.read).count();
                let meta = cache.get_feed_meta(&url);
                let name = meta
                    .as_ref()
                    .map(|m| m.title.clone())
                    .or_else(|| articles.first().map(|a| a.feed_name.clone()))
                    .unwrap_or_else(|| url.clone());
                folders.push(json!({
                    "id": url,
                    "name": name,
                    "url": meta.as_ref().map(|m| m.url.clone()),
                    "total": total,
                    "unread": unread,
                    "last_fetched": meta.map(|m| m.last_fetched),
                    "virtual": false,
                }));
            }

            json!({
                "ok": true,
                "folders": folders,
            })
        }
        Command::ListArticles {
            folder,
            offset,
            limit,
        } => {
            let limit = limit.unwrap_or(MAX_LIMIT).min(MAX_LIMIT);
            let articles = match folder_articles(cache, &folder) {
                Ok(articles) => sort_articles(articles),
                Err(error) => {
                    return json!({
                        "ok": false,
                        "error": error,
                    })
                }
            };

            let total = articles.len();
            let items: Vec<_> = articles
                .into_iter()
                .skip(offset)
                .take(limit)
                .map(|a| {
                    json!({
                        "hash": a.hash,
                        "title": a.title,
                        "link": a.link,
                        "published": a.published,
                        "feed_name": a.feed_name,
                        "read": a.read,
                    })
                })
                .collect();

            json!({
                "ok": true,
                "folder": folder,
                "offset": offset,
                "limit": limit,
                "total": total,
                "articles": items,
            })
        }
        Command::GetArticle { hash } => match cache.get_article(&hash) {
            Some(article) => json!({
                "ok": true,
                "article": article,
            }),
            None => json!({
                "ok": false,
                "error": format!("article not found: {}", hash),
            }),
        },
        Command::MarkRead { hash, read } => {
            if cache.get_article(&hash).is_none() {
                json!({
                    "ok": false,
                    "error": format!("article not found: {}", hash),
                })
            } else {
                cache.mark_read(&hash, read);
                json!({
                    "ok": true,
                    "hash": hash,
                    "read": read,
                })
            }
        }
        Command::MarkFolderRead { folder } => {
            let hashes = match folder_articles(cache, &folder) {
                Ok(articles) => articles.into_iter().map(|a| a.hash).collect::<Vec<_>>(),
                Err(error) => {
                    return json!({
                        "ok": false,
                        "error": error,
                    })
                }
            };
            for hash in &hashes {
                cache.mark_read(hash, true);
            }
            json!({
                "ok": true,
                "folder": folder,
                "updated": hashes.len(),
            })
        }
        Command::SearchArticles {
            query,
            folder,
            offset,
            limit,
        } => {
            let limit = limit.unwrap_or(MAX_LIMIT).min(MAX_LIMIT);
            let articles = match folder_articles(cache, &folder) {
                Ok(articles) => sort_articles(articles),
                Err(error) => {
                    return json!({
                        "ok": false,
                        "error": error,
                    })
                }
            };

            let query_lower = query.to_lowercase();
            let matched: Vec<_> = articles
                .into_iter()
                .filter(|a| {
                    let hay = format!(
                        "{} {} {}",
                        a.title.to_lowercase(),
                        a.description.to_lowercase(),
                        a.content.to_lowercase()
                    );
                    hay.contains(&query_lower)
                })
                .collect();

            let total = matched.len();
            let items: Vec<_> = matched
                .into_iter()
                .skip(offset)
                .take(limit)
                .map(|a| {
                    json!({
                        "hash": a.hash,
                        "title": a.title,
                        "link": a.link,
                        "published": a.published,
                        "feed_name": a.feed_name,
                        "read": a.read,
                    })
                })
                .collect();

            json!({
                "ok": true,
                "query": query,
                "folder": folder,
                "offset": offset,
                "limit": limit,
                "total": total,
                "articles": items,
            })
        }
        Command::Help => json!({
            "ok": true,
            "help": help_text(),
        }),
        Command::Quit => json!({
            "ok": true,
            "quit": true,
        }),
    }
}

fn folder_articles(cache: &Cache, folder: &str) -> Result<Vec<Article>, String> {
    match folder {
        "all" => Ok(cache.list_all_articles()),
        "unread" => Ok(cache.list_unread_articles()),
        url => {
            if !cache.list_feed_urls().iter().any(|f| f == url) {
                return Err(format!("unknown folder: {}", folder));
            }
            Ok(cache.list_feed_articles(url))
        }
    }
}

fn sort_articles(mut articles: Vec<Article>) -> Vec<Article> {
    articles.sort_by(|a, b| {
        let a_ts = datetime_sort_key(a.published.as_deref()).unwrap_or(i64::MIN);
        let b_ts = datetime_sort_key(b.published.as_deref()).unwrap_or(i64::MIN);
        match b_ts.cmp(&a_ts) {
            Ordering::Equal => a.title.cmp(&b.title),
            other => other,
        }
    });
    articles
}

pub fn help_text() -> String {
    [
        "CLI mode protocol (newline-delimited JSON):",
        "- Input: one JSON object per line on stdin",
        "- Output: one JSON object per line on stdout",
        "",
        "Commands:",
        "- {\"cmd\":\"list_folders\"}",
        "  Returns folders at root level. Includes virtual folders: all, unread.",
        "",
        "- {\"cmd\":\"list_articles\",\"folder\":\"all|unread|<feed_url>\",\"offset\":0,\"limit\":100}",
        "  Lists article summaries for a folder. limit is capped at 1000.",
        "",
        "- {\"cmd\":\"get_article\",\"hash\":\"<article_hash>\"}",
        "  Returns full article contents.",
        "",
        "- {\"cmd\":\"mark_read\",\"hash\":\"<article_hash>\",\"read\":true}",
        "  Sets an article read/unread flag in cache.",
        "",
        "- {\"cmd\":\"mark_folder_read\",\"folder\":\"all|unread|<feed_url>\"}",
        "  Marks all articles in a folder as read.",
        "",
        "- {\"cmd\":\"search_articles\",\"query\":\"<text>\",\"folder\":\"all\",\"offset\":0,\"limit\":100}",
        "  Case-insensitive substring search on title, description, and content.",
        "  folder defaults to \"all\". Returns matching articles with total match count.",
        "",
        "- {\"cmd\":\"help\"}",
        "  Returns this command documentation as a string.",
        "",
        "- {\"cmd\":\"quit\"}",
        "  Requests graceful shutdown of CLI mode.",
        "",
        "Response format:",
        "- Success: {\"ok\":true,...}",
        "- Error: {\"ok\":false,\"error\":\"...\"}",
    ]
    .join("\n")
}
