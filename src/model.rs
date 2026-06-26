use serde::{Deserialize, Serialize};

/// Maximum record size: 1 MB
pub const MAX_RECORD_SIZE: usize = 1_048_576;

/// Maximum announcement entry size: 256 bytes
pub const MAX_ANNOUNCEMENT_SIZE: usize = 256;

/// Known schemas
pub mod schema {
    pub const WIKI_ARTICLE: &str = "wiki/article";
    pub const RUST_CRATE: &str = "rust/crate";
    pub const LINK_MEDIA: &str = "link/media";
    pub const GENERIC_KV: &str = "generic/kv";

    pub fn known_schemas() -> &'static [&'static str] {
        &[WIKI_ARTICLE, RUST_CRATE, LINK_MEDIA, GENERIC_KV]
    }
}

/// How the content was scraped
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScrapeSource {
    Url,
    Feed,
    Api,
    Keyword,
}

impl Default for ScrapeSource {
    fn default() -> Self {
        Self::Url
    }
}

impl ScrapeSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Url => "url",
            Self::Feed => "feed",
            Self::Api => "api",
            Self::Keyword => "keyword",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "url" => Some(Self::Url),
            "feed" => Some(Self::Feed),
            "api" => Some(Self::Api),
            "keyword" => Some(Self::Keyword),
            _ => None,
        }
    }
}

impl std::fmt::Display for ScrapeSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// When a scrape job re-runs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RefreshPolicy {
    Once,
    Interval,
    OnChange,
}

impl Default for RefreshPolicy {
    fn default() -> Self {
        Self::Once
    }
}

impl RefreshPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::Interval => "interval",
            Self::OnChange => "on-change",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "once" => Some(Self::Once),
            "interval" => Some(Self::Interval),
            "on-change" => Some(Self::OnChange),
            _ => None,
        }
    }
}

impl std::fmt::Display for RefreshPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How long a record lives
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lifecycle {
    Ephemeral,
    Pinned,
}

impl Lifecycle {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ephemeral => "ephemeral",
            Self::Pinned => "pinned",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "ephemeral" => Some(Self::Ephemeral),
            "pinned" => Some(Self::Pinned),
            _ => None,
        }
    }
}

impl std::fmt::Display for Lifecycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A content record stored in Tier 3.
///
/// Fields are signed in declaration order (excluding `sig`) using
/// canonical length-prefixed encoding — see `trust::sign`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentRecord {
    pub id: String,
    pub source_url: String,
    pub source_hash: String,
    pub schema: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub body: String,
    pub created_at: u64,
    pub expires_at: u64,
    #[serde(default)]
    pub scrape_source: ScrapeSource,
    #[serde(default)]
    pub refresh_policy: RefreshPolicy,
    #[serde(default)]
    pub sig: String,
}

impl ContentRecord {
    /// Validate record size constraints.
    pub fn validate_size(&self) -> Result<(), String> {
        let json = serde_json::to_vec(self).map_err(|e| format!("serialization error: {}", e))?;
        if json.len() > MAX_RECORD_SIZE {
            return Err(format!(
                "record exceeds 1 MB limit ({} bytes)",
                json.len()
            ));
        }
        Ok(())
    }
}

/// An announcement entry for Tier 2.
///
/// Points from a record_id to the node that holds the content.
/// Fields are signed in declaration order (excluding `sig`) using
/// canonical length-prefixed encoding — see `trust::sign`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Announcement {
    pub record_id: String,
    pub source_hash: String,
    pub schema: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub holder_addr: String,
    pub expires_at: u64,
    #[serde(default)]
    pub sig: String,
}

impl Announcement {
    /// Validate announcement size constraints.
    pub fn validate_size(&self) -> Result<(), String> {
        let json = serde_json::to_vec(self).map_err(|e| format!("serialization error: {}", e))?;
        if json.len() > MAX_ANNOUNCEMENT_SIZE {
            return Err(format!(
                "announcement exceeds 256 byte limit ({} bytes)",
                json.len()
            ));
        }
        Ok(())
    }
}

/// A configured scrape job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrapeJob {
    pub name: String,
    pub source: ScrapeSource,
    pub target: String,
    #[serde(default)]
    pub transform: Option<String>,
    #[serde(default = "default_refresh")]
    pub refresh: RefreshPolicy,
    #[serde(default = "default_interval_secs")]
    pub interval_secs: u64,
    #[serde(default = "default_lifecycle")]
    pub lifecycle: Lifecycle,
    #[serde(default = "default_ttl_secs")]
    pub ttl_secs: u64,
    #[serde(default)]
    pub max_results: Option<u32>,
}

fn default_refresh() -> RefreshPolicy {
    RefreshPolicy::Once
}

fn default_interval_secs() -> u64 {
    3600
}

fn default_lifecycle() -> Lifecycle {
    Lifecycle::Ephemeral
}

fn default_ttl_secs() -> u64 {
    3600
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_record_serde_roundtrip() {
        let record = ContentRecord {
            id: "abc123".to_string(),
            source_url: "https://example.com".to_string(),
            source_hash: "hash123".to_string(),
            schema: schema::WIKI_ARTICLE.to_string(),
            tags: vec!["test".to_string()],
            body: "Hello world".to_string(),
            created_at: 1000,
            expires_at: 2000,
            scrape_source: ScrapeSource::Url,
            refresh_policy: RefreshPolicy::Once,
            sig: "sig123".to_string(),
        };
        let json = serde_json::to_string(&record).unwrap();
        let decoded: ContentRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.id, record.id);
        assert_eq!(decoded.source_url, record.source_url);
        assert_eq!(decoded.scrape_source, ScrapeSource::Url);
        assert_eq!(decoded.refresh_policy, RefreshPolicy::Once);
    }

    #[test]
    fn content_record_forward_compat() {
        // Extra fields should be ignored, not rejected
        let json = r#"{
            "id": "x",
            "source_url": "u",
            "source_hash": "h",
            "schema": "generic/kv",
            "tags": [],
            "body": "b",
            "created_at": 1,
            "expires_at": 2,
            "scrape_source": "url",
            "refresh_policy": "once",
            "sig": "",
            "future_field": "should be ignored"
        }"#;
        let decoded: ContentRecord = serde_json::from_str(json).unwrap();
        assert_eq!(decoded.id, "x");
    }

    #[test]
    fn content_record_missing_optional_fields() {
        // scrape_source, refresh_policy, sig, tags have serde(default)
        let json = r#"{
            "id": "x",
            "source_url": "u",
            "source_hash": "h",
            "schema": "generic/kv",
            "body": "b",
            "created_at": 1,
            "expires_at": 2
        }"#;
        let decoded: ContentRecord = serde_json::from_str(json).unwrap();
        assert_eq!(decoded.scrape_source, ScrapeSource::Url);
        assert_eq!(decoded.refresh_policy, RefreshPolicy::Once);
        assert_eq!(decoded.sig, "");
        assert!(decoded.tags.is_empty());
    }

    #[test]
    fn announcement_serde_roundtrip() {
        let ann = Announcement {
            record_id: "rid1".to_string(),
            source_hash: "shash1".to_string(),
            schema: schema::RUST_CRATE.to_string(),
            tags: vec!["async".to_string()],
            holder_addr: "1.2.3.4:7744".to_string(),
            expires_at: 9999,
            sig: "sigabc".to_string(),
        };
        let json = serde_json::to_string(&ann).unwrap();
        let decoded: Announcement = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.record_id, ann.record_id);
        assert_eq!(decoded.holder_addr, ann.holder_addr);
    }

    #[test]
    fn scrape_source_roundtrip() {
        for s in &["url", "feed", "api", "keyword"] {
            let src = ScrapeSource::from_str(s).unwrap();
            assert_eq!(src.as_str(), *s);
        }
        assert!(ScrapeSource::from_str("unknown").is_none());
    }

    #[test]
    fn refresh_policy_roundtrip() {
        for s in &["once", "interval", "on-change"] {
            let p = RefreshPolicy::from_str(s).unwrap();
            assert_eq!(p.as_str(), *s);
        }
        assert!(RefreshPolicy::from_str("unknown").is_none());
    }

    #[test]
    fn lifecycle_roundtrip() {
        for s in &["ephemeral", "pinned"] {
            let l = Lifecycle::from_str(s).unwrap();
            assert_eq!(l.as_str(), *s);
        }
        assert!(Lifecycle::from_str("unknown").is_none());
    }

    #[test]
    fn record_validate_size_ok() {
        let record = ContentRecord {
            id: "x".to_string(),
            source_url: "u".to_string(),
            source_hash: "h".to_string(),
            schema: "generic/kv".to_string(),
            tags: vec![],
            body: "small".to_string(),
            created_at: 1,
            expires_at: 2,
            scrape_source: ScrapeSource::Url,
            refresh_policy: RefreshPolicy::Once,
            sig: "".to_string(),
        };
        assert!(record.validate_size().is_ok());
    }

    #[test]
    fn known_schemas_list() {
        let schemas = schema::known_schemas();
        assert!(schemas.contains(&"wiki/article"));
        assert!(schemas.contains(&"rust/crate"));
        assert!(schemas.contains(&"link/media"));
        assert!(schemas.contains(&"generic/kv"));
    }
}
