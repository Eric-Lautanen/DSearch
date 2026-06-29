use crate::model::{ContentRecord, RefreshPolicy, ScrapeSource};
use crate::sanitize::sanitize_record;
use crate::storage::Store;
use crate::trust::sign::{compute_record_id, compute_source_hash};
use tracing::{debug, info};

/// Run a feed-source scrape job: fetch an RSS/Atom feed, parse entries, store records.
///
/// Parses the feed XML to extract entries with title, link, and description.
/// Each entry becomes a ContentRecord with the `generic/kv` schema.
pub async fn run_feed_job(
    store: &Store,
    name: &str,
    target: &str,
    ttl_secs: u64,
) -> Result<FeedScrapeResult, String> {
    let response = fetch_feed(target).await?;

    let entries = parse_feed(&response)?;

    let mut inserted = 0;
    let mut replaced = 0;
    let mut skipped = 0;

    for entry in &entries {
        let source_hash = compute_source_hash(entry.link.as_bytes());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let id = compute_record_id(
            entry.link.as_bytes(),
            source_hash.as_bytes(),
            b"generic/kv",
            b"",
            entry.body.as_bytes(),
            now.to_string().as_bytes(),
        );

        let mut record = ContentRecord {
            id,
            source_url: entry.link.clone(),
            source_hash,
            schema: "generic/kv".to_string(),
            tags: vec![format!("feed:{}", name)],
            body: entry.body.clone(),
            created_at: now,
            expires_at: now + ttl_secs,
            scrape_source: ScrapeSource::Feed,
            refresh_policy: RefreshPolicy::Interval,
            sig: String::new(),
        };

        record = sanitize_record(&record)?;
        match store.insert_record(&mut record)? {
            crate::storage::records::InsertResult::Inserted => inserted += 1,
            crate::storage::records::InsertResult::ReplacedNewer => replaced += 1,
            crate::storage::records::InsertResult::SkippedOlder => skipped += 1,
        }
    }

    info!(
        "Feed job '{}': {} entries, {} inserted, {} replaced, {} skipped",
        name,
        entries.len(),
        inserted,
        replaced,
        skipped
    );

    Ok(FeedScrapeResult {
        job_name: name.to_string(),
        total: entries.len(),
        inserted,
        replaced,
        skipped,
    })
}

/// A parsed feed entry.
struct FeedEntry {
    link: String,
    body: String,
}

/// Result of a feed scrape job.
pub struct FeedScrapeResult {
    pub job_name: String,
    pub total: usize,
    pub inserted: usize,
    pub replaced: usize,
    pub skipped: usize,
}

/// Parse an RSS/Atom feed XML into entries.
///
/// Simple XML parsing — extracts `<item>` (RSS) or `<entry>` (Atom) blocks,
/// then pulls `<link>` and `<description>`/`<summary>`/`<content>` from each.
fn parse_feed(xml: &str) -> Result<Vec<FeedEntry>, String> {
    let mut entries = Vec::new();

    // Try RSS <item> blocks
    for item_block in extract_xml_blocks(xml, "item") {
        let link = extract_xml_text(&item_block, "link").unwrap_or_default();
        let title = extract_xml_text(&item_block, "title").unwrap_or_default();
        let description = extract_xml_text(&item_block, "description")
            .or_else(|| extract_xml_text(&item_block, "content:encoded"))
            .unwrap_or_default();

        if !link.is_empty() {
            let body = if title.is_empty() {
                description
            } else if description.is_empty() {
                title
            } else {
                format!("{}\n\n{}", title, description)
            };
            entries.push(FeedEntry { link, body });
        }
    }

    // Try Atom <entry> blocks
    for entry_block in extract_xml_blocks(xml, "entry") {
        let link = extract_xml_text(&entry_block, "link")
            .or_else(|| {
                // Atom <link href="..." />
                entry_block
                    .lines()
                    .find(|l| l.contains("<link") && l.contains("href="))
                    .and_then(|l| {
                        let start = l.find("href=\"")? + 6;
                        let end = l[start..].find('"')? + start;
                        Some(l[start..end].to_string())
                    })
            })
            .unwrap_or_default();
        let title = extract_xml_text(&entry_block, "title").unwrap_or_default();
        let summary = extract_xml_text(&entry_block, "summary")
            .or_else(|| extract_xml_text(&entry_block, "content"))
            .unwrap_or_default();

        if !link.is_empty() {
            let body = if title.is_empty() {
                summary
            } else if summary.is_empty() {
                title
            } else {
                format!("{}\n\n{}", title, summary)
            };
            entries.push(FeedEntry { link, body });
        }
    }

    if entries.is_empty() && !xml.contains("<item") && !xml.contains("<entry") {
        return Err("no feed entries found (expected RSS <item> or Atom <entry>)".to_string());
    }

    Ok(entries)
}

/// Extract all XML blocks with the given tag name.
fn extract_xml_blocks(xml: &str, tag: &str) -> Vec<String> {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    let mut blocks = Vec::new();

    let mut search_from = 0;
    while let Some(start) = xml[search_from..].find(&open) {
        let abs_start = search_from + start;
        // Find end of opening tag
        let tag_end = xml[abs_start..].find('>').map(|i| abs_start + i + 1);
        let Some(content_start) = tag_end else { break };

        if let Some(end) = xml[content_start..].find(&close) {
            blocks.push(xml[content_start..content_start + end].to_string());
            search_from = content_start + end + close.len();
        } else {
            break;
        }
    }

    blocks
}

/// Extract text content from an XML tag within a block.
fn extract_xml_text(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);

    let start = block.find(&open)?;
    // Find end of opening tag (handles attributes and self-closing)
    let tag_end = block[start..].find('>')? + start + 1;
    // Check for self-closing tag
    if block[start..tag_end].ends_with("/>") {
        return None;
    }

    let end = block[tag_end..].find(&close)?;
    let text = block[tag_end..tag_end + end].trim().to_string();

    // Strip CDATA wrapper if present
    if let Some(stripped) = text
        .strip_prefix("<![CDATA[")
        .and_then(|s| s.strip_suffix("]]>"))
    {
        return Some(stripped.to_string());
    }

    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Fetch a feed URL using reqwest.
async fn fetch_feed(url: &str) -> Result<String, String> {
    debug!("Fetching feed: {}", url);
    let response = reqwest::get(url)
        .await
        .map_err(|e| format!("fetch feed {}: {}", url, e))?;
    let body = response
        .text()
        .await
        .map_err(|e| format!("read feed response: {}", e))?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rss_feed() {
        let xml = r#"<?xml version="1.0"?>
        <rss><channel><title>Test</title>
        <item>
            <title>Hello World</title>
            <link>https://example.com/1</link>
            <description>This is a test entry</description>
        </item>
        <item>
            <title>Second</title>
            <link>https://example.com/2</link>
            <description>Another entry</description>
        </item>
        </channel></rss>"#;

        let entries = parse_feed(xml).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].link, "https://example.com/1");
        assert!(entries[0].body.contains("Hello World"));
    }

    #[test]
    fn parse_atom_feed() {
        let xml = r#"<?xml version="1.0"?>
        <feed xmlns="http://www.w3.org/2005/Atom">
        <entry>
            <title>Atom Entry</title>
            <link href="https://example.com/atom1" />
            <summary>Atom summary</summary>
        </entry>
        </feed>"#;

        let entries = parse_feed(xml).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].link, "https://example.com/atom1");
    }

    #[test]
    fn parse_feed_cdata() {
        let xml = r#"<rss><channel>
        <item>
            <title>Test</title>
            <link>https://example.com/cdata</link>
            <description><![CDATA[<b>Bold content</b>]]></description>
        </item>
        </channel></rss>"#;

        let entries = parse_feed(xml).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].body.contains("<b>Bold content</b>"));
    }

    #[test]
    fn parse_feed_no_entries() {
        let xml = "<html>not a feed</html>";
        assert!(parse_feed(xml).is_err());
    }
}
