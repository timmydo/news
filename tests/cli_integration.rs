use redb::{Database, TableDefinition};
use serde::Serialize;
use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::tempdir;

const ARTICLES: TableDefinition<&str, &str> = TableDefinition::new("articles");
const FEEDS: TableDefinition<&str, &str> = TableDefinition::new("feeds");
const FEED_INDEX: TableDefinition<&str, &str> = TableDefinition::new("feed_index");

#[derive(Serialize)]
struct ArticleFixture {
    hash: String,
    title: String,
    link: String,
    description: String,
    content: String,
    published: Option<String>,
    feed_name: String,
    read: bool,
}

#[derive(Serialize)]
struct FeedMetaFixture {
    url: String,
    title: String,
    last_fetched: String,
}

#[test]
fn help_cli_includes_command_docs() {
    let output = Command::new(env!("CARGO_BIN_EXE_tn"))
        .arg("--help-cli")
        .output()
        .expect("run tn --help-cli");
    assert!(output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("stderr utf8");
    assert!(stderr.contains("list_folders"));
    assert!(stderr.contains("list_articles"));
    assert!(stderr.contains("get_article"));
}

#[test]
fn cli_mode_reads_json_commands_and_returns_json_lines() {
    let dir = tempdir().expect("tempdir");
    let cache_path = dir.path().join("test.redb");
    seed_cache(&cache_path);

    let mut child = Command::new(env!("CARGO_BIN_EXE_tn"))
        .args(["--cli", "--cache"])
        .arg(&cache_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn tn --cli");

    let input = concat!(
        "{\"cmd\":\"list_folders\"}\n",
        "{\"cmd\":\"list_articles\",\"folder\":\"https://example.com/feed\"}\n",
        "{\"cmd\":\"get_article\",\"hash\":\"a1\"}\n",
        "{\"cmd\":\"mark_read\",\"hash\":\"a1\",\"read\":true}\n",
        "{\"cmd\":\"get_article\",\"hash\":\"a1\"}\n",
        "{\"cmd\":\"quit\"}\n"
    );
    let stdin = child.stdin.as_mut().expect("stdin");
    stdin.write_all(input.as_bytes()).expect("write stdin");

    let output = child.wait_with_output().expect("wait output");
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 6);

    let responses: Vec<Value> = lines
        .iter()
        .map(|line| serde_json::from_str(line).expect("valid json line"))
        .collect();

    assert_eq!(responses[0]["ok"], Value::Bool(true));
    let folders = responses[0]["folders"].as_array().expect("folders array");
    assert!(folders
        .iter()
        .any(|f| f["id"] == Value::String("https://example.com/feed".to_string())));

    assert_eq!(responses[1]["ok"], Value::Bool(true));
    assert_eq!(responses[1]["total"], Value::from(2u64));

    assert_eq!(responses[2]["ok"], Value::Bool(true));
    assert_eq!(
        responses[2]["article"]["title"],
        Value::String("First".to_string())
    );
    assert_eq!(responses[2]["article"]["read"], Value::Bool(false));

    assert_eq!(responses[3]["ok"], Value::Bool(true));
    assert_eq!(responses[3]["read"], Value::Bool(true));

    assert_eq!(responses[4]["ok"], Value::Bool(true));
    assert_eq!(responses[4]["article"]["read"], Value::Bool(true));

    assert_eq!(responses[5]["ok"], Value::Bool(true));
    assert_eq!(responses[5]["quit"], Value::Bool(true));
}

fn seed_cache(path: &std::path::Path) {
    let db = Database::create(path).expect("create db");
    let write_txn = db.begin_write().expect("begin write");

    {
        let mut articles = write_txn.open_table(ARTICLES).expect("articles table");
        let a1 = ArticleFixture {
            hash: "a1".to_string(),
            title: "First".to_string(),
            link: "https://example.com/1".to_string(),
            description: "desc1".to_string(),
            content: "content1".to_string(),
            published: Some("2025-01-02 03:04:05".to_string()),
            feed_name: "Example Feed".to_string(),
            read: false,
        };
        let a2 = ArticleFixture {
            hash: "a2".to_string(),
            title: "Second".to_string(),
            link: "https://example.com/2".to_string(),
            description: "desc2".to_string(),
            content: "content2".to_string(),
            published: Some("2025-01-01 03:04:05".to_string()),
            feed_name: "Example Feed".to_string(),
            read: true,
        };
        let a1_json = serde_json::to_string(&a1).expect("serialize a1");
        let a2_json = serde_json::to_string(&a2).expect("serialize a2");
        articles.insert("a1", a1_json.as_str()).expect("insert a1");
        articles.insert("a2", a2_json.as_str()).expect("insert a2");
    }

    {
        let mut feeds = write_txn.open_table(FEEDS).expect("feeds table");
        let feed_meta = FeedMetaFixture {
            url: "https://example.com/feed".to_string(),
            title: "Example Feed".to_string(),
            last_fetched: "2025-01-03 10:00:00".to_string(),
        };
        let feed_meta_json = serde_json::to_string(&feed_meta).expect("serialize feed meta");
        feeds
            .insert("https://example.com/feed", feed_meta_json.as_str())
            .expect("insert feed");
    }

    {
        let mut index = write_txn.open_table(FEED_INDEX).expect("feed index table");
        let hashes_json = serde_json::to_string(&vec!["a1", "a2"]).expect("serialize hashes");
        index
            .insert("https://example.com/feed", hashes_json.as_str())
            .expect("insert feed index");
    }

    write_txn.commit().expect("commit");
}
