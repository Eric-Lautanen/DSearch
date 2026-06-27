pub mod records;
pub mod index;
pub mod expiry;
pub mod quota;
pub mod migrations;
pub mod tier2_limiter;

use redb::{Database, TableDefinition};
use std::path::Path;
use std::sync::Arc;
use crate::config::StorageConfig;
use crate::model::{ContentRecord, Announcement};

/// All table definitions used by the store.
const RECORDS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("records");
const ANNOUNCEMENTS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("announcements");
const SOURCE_INDEX_TABLE: TableDefinition<&str, &str> = TableDefinition::new("source_index");
const PINS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("pins");
const ROUTING_TABLE: TableDefinition<&str, &str> = TableDefinition::new("routing");
const PEERS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("peers");
const BANNED_PEERS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("banned_peers");
const META_TABLE: TableDefinition<&str, u64> = TableDefinition::new("meta");
const INVERTED_INDEX_TABLE: TableDefinition<&str, &str> = TableDefinition::new("inverted_index");

/// Open the store database, creating it if needed.
/// Runs schema version check and creates all tables.
pub fn open_store(data_dir: &Path) -> Result<Arc<Database>, String> {
    std::fs::create_dir_all(data_dir)
        .map_err(|e| format!("create data dir: {}", e))?;

    let db_path = data_dir.join("store.redb");
    let db = Database::builder()
        .create(&db_path)
        .map_err(|e| format!("open store.redb: {}", e))?;

    // Create all tables
    let write_tx = db.begin_write().map_err(|e| format!("init write tx: {}", e))?;
    write_tx.open_table(RECORDS_TABLE).map_err(|e| format!("create records table: {}", e))?;
    write_tx.open_table(ANNOUNCEMENTS_TABLE).map_err(|e| format!("create announcements table: {}", e))?;
    write_tx.open_table(SOURCE_INDEX_TABLE).map_err(|e| format!("create source_index table: {}", e))?;
    write_tx.open_table(PINS_TABLE).map_err(|e| format!("create pins table: {}", e))?;
    write_tx.open_table(ROUTING_TABLE).map_err(|e| format!("create routing table: {}", e))?;
    write_tx.open_table(PEERS_TABLE).map_err(|e| format!("create peers table: {}", e))?;
    write_tx.open_table(BANNED_PEERS_TABLE).map_err(|e| format!("create banned_peers table: {}", e))?;
    write_tx.open_table(META_TABLE).map_err(|e| format!("create meta table: {}", e))?;
    write_tx.open_table(INVERTED_INDEX_TABLE).map_err(|e| format!("create inverted_index table: {}", e))?;
    write_tx.commit().map_err(|e| format!("init commit: {}", e))?;

    // Check schema version and run migrations
    migrations::check_and_migrate(&db)?;

    Ok(Arc::new(db))
}

/// High-level store operations that coordinate across tables.
pub struct Store {
    db: Arc<Database>,
    config: StorageConfig,
}

impl Store {
    pub fn new(db: Arc<Database>, config: StorageConfig) -> Self {
        Self { db, config }
    }

    /// Insert a record with quota check, dedup, and index update.
    pub fn insert_record(&self, record: &ContentRecord) -> Result<records::InsertResult, String> {
        // Check quota
        let json_len = serde_json::to_vec(record)
            .map_err(|e| format!("serialize for quota check: {}", e))?
            .len() as u64;
        quota::check_quota(&self.db, &self.config, json_len)?;

        // Insert with dedup
        let result = records::insert_record(&self.db, record)?;

        // Update inverted index if inserted or replaced
        match &result {
            records::InsertResult::Inserted | records::InsertResult::ReplacedNewer => {
                index::index_record(&self.db, &record.id, &record.schema, &record.tags)?;
            }
            records::InsertResult::SkippedOlder => {}
        }

        Ok(result)
    }

    /// Get a record by ID.
    pub fn get_record(&self, id: &str) -> Result<Option<ContentRecord>, String> {
        records::get_record(&self.db, id)
    }

    /// List records, optionally filtered by schema.
    pub fn list_records(&self, schema: Option<&str>, limit: usize) -> Result<Vec<ContentRecord>, String> {
        records::list_records(&self.db, schema, limit)
    }

    /// Delete a record by ID, also removes from index and source_index.
    pub fn delete_record(&self, id: &str) -> Result<bool, String> {
        // Get record for deindexing before delete
        let record = records::get_record(&self.db, id)?;
        let deleted = records::delete_record(&self.db, id)?;
        if deleted {
            if let Some(r) = record {
                index::deindex_record(&self.db, &r.id, &r.schema, &r.tags)?;
            }
        }
        Ok(deleted)
    }

    /// Pin a record.
    pub fn pin_record(&self, id: &str) -> Result<bool, String> {
        records::pin_record(&self.db, id)
    }

    /// Unpin a record.
    pub fn unpin_record(&self, id: &str) -> Result<bool, String> {
        records::unpin_record(&self.db, id)
    }

    /// Check if a record is pinned.
    pub fn is_pinned(&self, id: &str) -> Result<bool, String> {
        records::is_pinned(&self.db, id)
    }

    /// Insert an announcement.
    pub fn insert_announcement(&self, ann: &Announcement) -> Result<(), String> {
        records::insert_announcement(&self.db, ann)
    }

    /// List announcements.
    pub fn list_announcements(&self, record_id: Option<&str>) -> Result<Vec<Announcement>, String> {
        records::list_announcements(&self.db, record_id)
    }

    /// Search the inverted index.
    pub fn search_index(
        &self,
        schema: &str,
        tag_key: Option<&str>,
        tag_value: Option<&str>,
    ) -> Result<Vec<String>, String> {
        index::search_index(&self.db, schema, tag_key, tag_value)
    }

    /// Search local Tier 3 records using the query language.
    /// Returns records matching the query, ranked by score.
    pub fn search_records(
        &self,
        query_str: &str,
        limit: usize,
    ) -> Result<Vec<ContentRecord>, String> {
        let parsed = crate::search::query::parse_query(query_str);
        let effective_limit = parsed.limit.unwrap_or(limit);

        // If schema filter is specified, use it to narrow the scan
        let schema_filter = parsed.fields.get("schema").map(|s| s.as_str());

        // Scan all records (or filtered by schema) and match against query
        let all_records = records::list_records(&self.db, schema_filter, effective_limit * 10)?;

        let mut matched: Vec<(ContentRecord, f64)> = Vec::new();
        for record in all_records {
            if crate::search::query::matches_query(&record, &parsed) {
                let score = crate::search::query::score_record(&record, &parsed, 1);
                matched.push((record, score));
            }
        }

        // Sort by score descending
        matched.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Apply limit
        Ok(matched.into_iter().take(effective_limit).map(|(r, _)| r).collect())
    }

    /// Get record count.
    pub fn record_count(&self) -> Result<u64, String> {
        records::record_count(&self.db)
    }

    /// Get records size in bytes.
    pub fn records_size_bytes(&self) -> Result<u64, String> {
        records::records_size_bytes(&self.db)
    }

    /// Run a single expiry sweep.
    pub fn sweep_once(&self) -> Result<(usize, usize), String> {
        expiry::sweep_once(&self.db)
    }

    /// Start the background expiry sweeper.
    pub fn start_expiry_sweeper(&self, interval: std::time::Duration) -> tokio::task::JoinHandle<()> {
        expiry::start_expiry_sweeper(self.db.clone(), interval)
    }

    /// Get the underlying database reference.
    pub fn db(&self) -> &Arc<Database> {
        &self.db
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ScrapeSource, RefreshPolicy, schema};
    use tempfile::TempDir;

    fn open_test_store() -> (TempDir, Store) {
        let dir = TempDir::new().unwrap();
        let db = open_store(dir.path()).unwrap();
        let config = StorageConfig::default();
        let store = Store::new(db, config);
        (dir, store)
    }

    fn make_record(id: &str, source_hash: &str, created_at: u64, expires_at: u64) -> ContentRecord {
        ContentRecord {
            id: id.to_string(),
            source_url: "https://example.com".to_string(),
            source_hash: source_hash.to_string(),
            schema: schema::WIKI_ARTICLE.to_string(),
            tags: vec!["category:networking".to_string()],
            body: "Hello world".to_string(),
            created_at,
            expires_at,
            scrape_source: ScrapeSource::Url,
            refresh_policy: RefreshPolicy::Once,
            sig: "".to_string(),
        }
    }

    #[test]
    fn store_insert_and_list() {
        let (_dir, store) = open_test_store();
        let r1 = make_record("r1", "sh1", 1000, 2000);
        store.insert_record(&r1).unwrap();

        let records = store.list_records(None, 100).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "r1");
    }

    #[test]
    fn store_dedup_keeps_newer() {
        let (_dir, store) = open_test_store();
        let older = make_record("r1", "sh1", 1000, 2000);
        let newer = make_record("r2", "sh1", 2000, 3000);

        store.insert_record(&older).unwrap();
        let result = store.insert_record(&newer).unwrap();
        assert!(matches!(result, records::InsertResult::ReplacedNewer));

        let records = store.list_records(None, 100).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "r2");
    }

    #[test]
    fn store_delete_removes_everywhere() {
        let (_dir, store) = open_test_store();
        let r1 = make_record("r1", "sh1", 1000, 2000);
        store.insert_record(&r1).unwrap();

        assert!(store.delete_record("r1").unwrap());
        assert!(store.get_record("r1").unwrap().is_none());
        assert_eq!(store.list_records(None, 100).unwrap().len(), 0);
    }

    #[test]
    fn store_pin_unpin() {
        let (_dir, store) = open_test_store();
        let r1 = make_record("r1", "sh1", 1000, 2000);
        store.insert_record(&r1).unwrap();

        assert!(store.pin_record("r1").unwrap());
        assert!(store.is_pinned("r1").unwrap());
        assert!(store.unpin_record("r1").unwrap());
        assert!(!store.is_pinned("r1").unwrap());
    }

    #[test]
    fn store_search_by_tag() {
        let (_dir, store) = open_test_store();
        let r1 = make_record("r1", "sh1", 1000, 2000);
        store.insert_record(&r1).unwrap();

        let results = store.search_index("wiki/article", Some("category"), Some("networking")).unwrap();
        assert!(results.contains(&"r1".to_string()));
    }

    #[test]
    fn store_expiry_sweep() {
        let (_dir, store) = open_test_store();
        let r1 = make_record("r1", "sh1", 1000, 100); // expired
        store.insert_record(&r1).unwrap();

        let (removed, _) = store.sweep_once().unwrap();
        assert_eq!(removed, 1);
        assert!(store.get_record("r1").unwrap().is_none());
    }

    #[test]
    fn store_search_records() {
        let (_dir, store) = open_test_store();
        let r1 = make_record("r1", "sh1", 1000, 2000);
        let mut r2 = make_record("r2", "sh2", 1000, 2000);
        r2.schema = crate::model::schema::RUST_CRATE.to_string();
        r2.body = "Tokio async runtime benchmarks".to_string();

        store.insert_record(&r1).unwrap();
        store.insert_record(&r2).unwrap();

        // Simple text search
        let results = store.search_records("hello", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "r1");

        // Schema filter via query language
        let results = store.search_records("schema:rust/crate", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "r2");

        // Search across body
        let results = store.search_records("async", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "r2");
    }
}
