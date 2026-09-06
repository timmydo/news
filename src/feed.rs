use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

pub const DISPLAY_DATETIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

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
    let body = if crate::td_fetch::available() {
        // Inside a td jail that carries `sockets=fetch`: the fetch service
        // holds the network, this program holds a socket (td's
        // APPLICATIONS.md §W.8). The service names its refusals, and a
        // status is an answer to be shown, not a transport error.
        let response = crate::td_fetch::get(url, &[], None, None)
            .map_err(|e| format!("fetch {}: {}", url, e))?;
        if !(200..300).contains(&response.status) {
            return Err(format!("fetch {}: HTTP {}", url, response.status));
        }
        String::from_utf8(response.body).map_err(|e| format!("read body from {}: {}", url, e))?
    } else {
        let resp = ureq::get(url)
            .timeout(std::time::Duration::from_secs(30))
            .call()
            .map_err(|e| format!("fetch {}: {}", url, e))?;
        resp.into_string()
            .map_err(|e| format!("read body from {}: {}", url, e))?
    };

    parse_feed(&body, feed_name)
}

/// Parse RSS/Atom XML into articles.
fn parse_feed(xml: &str, feed_name: &str) -> Result<(String, Vec<Article>), String> {
    // Detect feed type by looking for <feed (Atom) vs <rss or <channel (RSS)
    let trimmed = xml.trim_start();
    if trimmed.contains("<feed") && !trimmed.contains("<rss") {
        parse_atom(xml, feed_name)
    } else {
        parse_rss(xml, feed_name)
    }
}

/// Parse an RSS 2.0 feed.
fn parse_rss(xml: &str, feed_name: &str) -> Result<(String, Vec<Article>), String> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();

    let mut feed_title = String::new();
    let mut articles = Vec::new();

    // State tracking
    let mut in_channel = false;
    let mut in_item = false;
    let mut current_tag = String::new();

    // Item fields
    let mut item_title = String::new();
    let mut item_link = String::new();
    let mut item_description = String::new();
    let mut item_content = String::new();
    let mut item_pub_date = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "channel" => in_channel = true,
                    "item" => {
                        in_item = true;
                        item_title.clear();
                        item_link.clear();
                        item_description.clear();
                        item_content.clear();
                        item_pub_date.clear();
                    }
                    _ => current_tag = name,
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "item" {
                    let link = item_link.trim().to_string();
                    let title = item_title.trim().to_string();
                    let hash = article_hash(&title, &link);
                    articles.push(Article {
                        hash,
                        title,
                        link,
                        description: item_description.trim().to_string(),
                        content: item_content.trim().to_string(),
                        published: normalize_optional_datetime(&item_pub_date),
                        feed_name: feed_name.to_string(),
                        read: false,
                    });
                    in_item = false;
                } else if name == "channel" {
                    in_channel = false;
                }
                current_tag.clear();
            }
            Ok(Event::Text(ref e)) => {
                let text = e.unescape().unwrap_or_default().to_string();
                rss_apply_text(
                    &text,
                    in_item,
                    in_channel,
                    &current_tag,
                    &mut item_title,
                    &mut item_link,
                    &mut item_description,
                    &mut item_content,
                    &mut item_pub_date,
                    &mut feed_title,
                );
            }
            Ok(Event::CData(ref e)) => {
                let text = String::from_utf8_lossy(e.as_ref()).to_string();
                rss_apply_text(
                    &text,
                    in_item,
                    in_channel,
                    &current_tag,
                    &mut item_title,
                    &mut item_link,
                    &mut item_description,
                    &mut item_content,
                    &mut item_pub_date,
                    &mut feed_title,
                );
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML parse error: {}", e)),
            _ => {}
        }
        buf.clear();
    }

    if feed_title.is_empty() {
        feed_title = feed_name.to_string();
    }

    Ok((feed_title, articles))
}

#[allow(clippy::too_many_arguments)]
fn rss_apply_text(
    text: &str,
    in_item: bool,
    in_channel: bool,
    current_tag: &str,
    item_title: &mut String,
    item_link: &mut String,
    item_description: &mut String,
    item_content: &mut String,
    item_pub_date: &mut String,
    feed_title: &mut String,
) {
    if in_item {
        match current_tag {
            "title" => item_title.push_str(text),
            "link" => item_link.push_str(text),
            "description" => item_description.push_str(text),
            "content:encoded" | "content" => item_content.push_str(text),
            "pubDate" | "dc:date" => item_pub_date.push_str(text),
            _ => {}
        }
    } else if in_channel && current_tag == "title" {
        feed_title.push_str(text);
    }
}

/// Parse an Atom feed.
fn parse_atom(xml: &str, feed_name: &str) -> Result<(String, Vec<Article>), String> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();

    let mut feed_title = String::new();
    let mut articles = Vec::new();

    let mut in_feed = false;
    let mut in_entry = false;
    let mut current_tag = String::new();

    let mut entry_title = String::new();
    let mut entry_link = String::new();
    let mut entry_summary = String::new();
    let mut entry_content = String::new();
    let mut entry_published = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "feed" => in_feed = true,
                    "entry" => {
                        in_entry = true;
                        entry_title.clear();
                        entry_link.clear();
                        entry_summary.clear();
                        entry_content.clear();
                        entry_published.clear();
                    }
                    "link" => {
                        // Atom links are attributes: <link href="..." rel="alternate" />
                        let mut href = String::new();
                        let mut rel = String::new();
                        for attr in e.attributes().flatten() {
                            let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                            let val = String::from_utf8_lossy(&attr.value).to_string();
                            if key == "href" {
                                href = val;
                            } else if key == "rel" {
                                rel = val;
                            }
                        }
                        // Use alternate link, or any link if no rel specified
                        if in_entry
                            && (rel.is_empty() || rel == "alternate")
                            && entry_link.is_empty()
                        {
                            entry_link = href;
                        }
                    }
                    _ => current_tag = name,
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "entry" {
                    let link = entry_link.trim().to_string();
                    let title = entry_title.trim().to_string();
                    let hash = article_hash(&title, &link);
                    articles.push(Article {
                        hash,
                        title,
                        link,
                        description: entry_summary.trim().to_string(),
                        content: entry_content.trim().to_string(),
                        published: normalize_optional_datetime(&entry_published),
                        feed_name: feed_name.to_string(),
                        read: false,
                    });
                    in_entry = false;
                } else if name == "feed" {
                    in_feed = false;
                }
                current_tag.clear();
            }
            Ok(Event::Text(ref e)) => {
                let text = e.unescape().unwrap_or_default().to_string();
                atom_apply_text(
                    &text,
                    in_entry,
                    in_feed,
                    &current_tag,
                    &mut entry_title,
                    &mut entry_summary,
                    &mut entry_content,
                    &mut entry_published,
                    &mut feed_title,
                );
            }
            Ok(Event::CData(ref e)) => {
                let text = String::from_utf8_lossy(e.as_ref()).to_string();
                atom_apply_text(
                    &text,
                    in_entry,
                    in_feed,
                    &current_tag,
                    &mut entry_title,
                    &mut entry_summary,
                    &mut entry_content,
                    &mut entry_published,
                    &mut feed_title,
                );
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML parse error: {}", e)),
            _ => {}
        }
        buf.clear();
    }

    if feed_title.is_empty() {
        feed_title = feed_name.to_string();
    }

    Ok((feed_title, articles))
}

#[allow(clippy::too_many_arguments)]
fn atom_apply_text(
    text: &str,
    in_entry: bool,
    in_feed: bool,
    current_tag: &str,
    entry_title: &mut String,
    entry_summary: &mut String,
    entry_content: &mut String,
    entry_published: &mut String,
    feed_title: &mut String,
) {
    if in_entry {
        match current_tag {
            "title" => entry_title.push_str(text),
            "summary" => entry_summary.push_str(text),
            "content" => entry_content.push_str(text),
            "published" | "updated" => {
                if entry_published.is_empty() {
                    entry_published.push_str(text);
                }
            }
            _ => {}
        }
    } else if in_feed && current_tag == "title" {
        feed_title.push_str(text);
    }
}

/// Compute a deduplication hash for an article (matching feed2maildir's approach).
pub fn article_hash(title: &str, link: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    title.hash(&mut hasher);
    link.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub fn now_local_datetime_string() -> String {
    Local::now().format(DISPLAY_DATETIME_FORMAT).to_string()
}

pub fn normalize_datetime_to_local(input: &str) -> Option<String> {
    parse_datetime_to_local(input).map(|dt| dt.format(DISPLAY_DATETIME_FORMAT).to_string())
}

pub fn datetime_sort_key(input: Option<&str>) -> Option<i64> {
    parse_datetime_to_local(input?).map(|dt| dt.timestamp())
}

fn normalize_optional_datetime(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        None
    } else {
        normalize_datetime_to_local(trimmed)
    }
}

fn parse_datetime_to_local(input: &str) -> Option<DateTime<Local>> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }

    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Local));
    }
    if let Ok(dt) = DateTime::parse_from_rfc2822(s) {
        return Some(dt.with_timezone(&Local));
    }

    for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(s, fmt) {
            if let Some(local) = Local
                .from_local_datetime(&naive)
                .single()
                .or_else(|| Local.from_local_datetime(&naive).earliest())
            {
                return Some(local);
            }
        }
    }

    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let naive = date.and_hms_opt(0, 0, 0)?;
        return Local
            .from_local_datetime(&naive)
            .single()
            .or_else(|| Local.from_local_datetime(&naive).earliest());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rss_feed() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Test Feed</title>
    <link>https://example.com</link>
    <item>
      <title>First Post</title>
      <link>https://example.com/1</link>
      <description>Hello world</description>
      <pubDate>Mon, 01 Jan 2024 00:00:00 GMT</pubDate>
    </item>
    <item>
      <title>Second Post</title>
      <link>https://example.com/2</link>
      <description>Another post</description>
    </item>
  </channel>
</rss>"#;
        let (title, articles) = parse_feed(xml, "Test").unwrap();
        assert_eq!(title, "Test Feed");
        assert_eq!(articles.len(), 2);
        assert_eq!(articles[0].title, "First Post");
        assert_eq!(articles[0].link, "https://example.com/1");
        assert_eq!(articles[0].description, "Hello world");
        assert!(articles[0].published.is_some());
        assert_eq!(articles[1].title, "Second Post");
        assert!(articles[1].published.is_none());
        assert!(!articles[0].read);
        assert_eq!(articles[0].feed_name, "Test");
    }

    #[test]
    fn parse_atom_feed() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Atom Feed</title>
  <entry>
    <title>Atom Entry</title>
    <link href="https://example.com/atom/1" rel="alternate" />
    <summary>An atom summary</summary>
    <content>Full content here</content>
    <published>2024-01-01T00:00:00Z</published>
  </entry>
</feed>"#;
        let (title, articles) = parse_feed(xml, "AtomTest").unwrap();
        assert_eq!(title, "Atom Feed");
        assert_eq!(articles.len(), 1);
        assert_eq!(articles[0].title, "Atom Entry");
        assert_eq!(articles[0].link, "https://example.com/atom/1");
        assert_eq!(articles[0].description, "An atom summary");
        assert_eq!(articles[0].content, "Full content here");
        assert!(articles[0].published.is_some());
    }

    #[test]
    fn parse_rss_with_cdata() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>CDATA Feed</title>
    <item>
      <title><![CDATA[Post with <special> chars]]></title>
      <link>https://example.com/cdata</link>
      <description><![CDATA[<p>HTML content</p>]]></description>
    </item>
  </channel>
</rss>"#;
        let (_, articles) = parse_feed(xml, "Test").unwrap();
        assert_eq!(articles.len(), 1);
        assert_eq!(articles[0].title, "Post with <special> chars");
        assert_eq!(articles[0].description, "<p>HTML content</p>");
    }

    #[test]
    fn article_hash_deterministic() {
        let h1 = article_hash("title", "link");
        let h2 = article_hash("title", "link");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16);
    }

    #[test]
    fn article_hash_differs() {
        let h1 = article_hash("title1", "link");
        let h2 = article_hash("title2", "link");
        assert_ne!(h1, h2);
    }
}
