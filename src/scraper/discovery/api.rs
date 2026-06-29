use crate::model::{ContentRecord, RefreshPolicy, ScrapeSource};
use crate::sanitize::sanitize_record;
use crate::storage::Store;
use crate::trust::sign::{compute_record_id, compute_source_hash};
use tracing::{debug, info};

/// Run an API-source scrape job: query an API endpoint, parse JSON response, store records.
///
/// The API endpoint should return a JSON array of objects. Each object is
/// converted into a ContentRecord with the `generic/kv` schema.
pub async fn run_api_job(
    store: &Store,
    name: &str,
    target: &str,
    ttl_secs: u64,
) -> Result<ApiScrapeResult, String> {
    let response = fetch_api(target).await?;

    let records = parse_api_response(&response)?;

    let mut inserted = 0;
    let mut replaced = 0;
    let mut skipped = 0;

    for item in &records {
        let source_hash = compute_source_hash(item.source_url.as_bytes());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let id = compute_record_id(
            item.source_url.as_bytes(),
            source_hash.as_bytes(),
            item.schema.as_bytes(),
            b"",
            item.body.as_bytes(),
            now.to_string().as_bytes(),
        );

        let mut record = ContentRecord {
            id,
            source_url: item.source_url.clone(),
            source_hash,
            schema: item.schema.clone(),
            tags: item.tags.clone(),
            body: item.body.clone(),
            created_at: now,
            expires_at: now + ttl_secs,
            scrape_source: ScrapeSource::Api,
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
        "API job '{}': {} inserted, {} replaced, {} skipped",
        name, inserted, replaced, skipped
    );

    Ok(ApiScrapeResult {
        job_name: name.to_string(),
        total: records.len(),
        inserted,
        replaced,
        skipped,
    })
}

/// A parsed item from an API response.
struct ApiItem {
    source_url: String,
    schema: String,
    tags: Vec<String>,
    body: String,
}

/// Result of an API scrape job.
pub struct ApiScrapeResult {
    pub job_name: String,
    pub total: usize,
    pub inserted: usize,
    pub replaced: usize,
    pub skipped: usize,
}

/// Parse an API JSON response into a list of items.
///
/// Expects a JSON array of objects, each with at least a `url` or `source_url` field
/// and a `body` or `content` field. Falls back to serializing the entire object as
/// the body if no body field is found.
fn parse_api_response(response: &str) -> Result<Vec<ApiItem>, String> {
    let value: serde_json::Value =
        serde_json::from_str(response).map_err(|e| format!("parse API response: {}", e))?;

    let items = match value {
        serde_json::Value::Array(arr) => arr,
        serde_json::Value::Object(_) => vec![value],
        _ => return Err("API response must be a JSON array or object".to_string()),
    };

    let mut result = Vec::new();
    for item in &items {
        let source_url = item
            .get("url")
            .or_else(|| item.get("source_url"))
            .and_then(|v| v.as_str())
            .unwrap_or("api://unknown")
            .to_string();

        let schema = item
            .get("schema")
            .and_then(|v| v.as_str())
            .unwrap_or("generic/kv")
            .to_string();

        let tags = item
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let body_str = item
            .get("body")
            .or_else(|| item.get("content"))
            .and_then(|v| v.as_str());

        let body = if let Some(s) = body_str {
            s.to_string()
        } else {
            serde_json::to_string(item).unwrap_or_default()
        };

        result.push(ApiItem {
            source_url,
            schema,
            tags,
            body,
        });
    }

    Ok(result)
}

/// Fetch an API endpoint using reqwest.
async fn fetch_api(url: &str) -> Result<String, String> {
    debug!("Fetching API: {}", url);
    let response = reqwest::get(url)
        .await
        .map_err(|e| format!("fetch API {}: {}", url, e))?;
    let body = response
        .text()
        .await
        .map_err(|e| format!("read API response: {}", e))?;
    Ok(body)
}
