pub mod expiry;
pub mod index;
pub mod migrations;
pub mod quota;
pub mod records;
pub mod tier2_limiter;
use crate::config::StorageConfig;
use crate::model::ContentRecord;
use crate::search::cache::SearchCache;
use redb::{Database, TableDefinition};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

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
    std::fs::create_dir_all(data_dir).map_err(|e| format!("create data dir: {}", e))?;

    let db_path = data_dir.join("store.redb");
    let db = Database::builder()
        .create(&db_path)
        .map_err(|e| format!("open store.redb: {}", e))?;

    // Create all tables
    let write_tx = db
        .begin_write()
        .map_err(|e| format!("init write tx: {}", e))?;
    write_tx
        .open_table(RECORDS_TABLE)
        .map_err(|e| format!("create records table: {}", e))?;
    write_tx
        .open_table(ANNOUNCEMENTS_TABLE)
        .map_err(|e| format!("create announcements table: {}", e))?;
    write_tx
        .open_table(SOURCE_INDEX_TABLE)
        .map_err(|e| format!("create source_index table: {}", e))?;
    write_tx
        .open_table(PINS_TABLE)
        .map_err(|e| format!("create pins table: {}", e))?;
    write_tx
        .open_table(ROUTING_TABLE)
        .map_err(|e| format!("create routing table: {}", e))?;
    write_tx
        .open_table(PEERS_TABLE)
        .map_err(|e| format!("create peers table: {}", e))?;
    write_tx
        .open_table(BANNED_PEERS_TABLE)
        .map_err(|e| format!("create banned_peers table: {}", e))?;
    write_tx
        .open_table(META_TABLE)
        .map_err(|e| format!("create meta table: {}", e))?;
    write_tx
        .open_table(INVERTED_INDEX_TABLE)
        .map_err(|e| format!("create inverted_index table: {}", e))?;
    write_tx
        .commit()
        .map_err(|e| format!("init commit: {}", e))?;

    // Check schema version and run migrations
    migrations::check_and_migrate(&db)?;

    Ok(Arc::new(db))
}

/// High-level store operations that coordinate across tables.
pub struct Store {
    db: Arc<Database>,
    config: StorageConfig,
    signing_key: Option<Arc<ed25519_dalek::SigningKey>>,
    search_cache: SearchCache,
    tier2_limiter: crate::storage::tier2_limiter::Tier2Limiter,
    bandwidth_account: crate::node::relay::RelayBandwidthAccount,
}

impl Store {
    pub fn new(db: Arc<Database>, config: StorageConfig) -> Self {
        let search_cache = SearchCache::new(Duration::from_secs(30), 256);
        let tier2_limiter =
            crate::storage::tier2_limiter::Tier2Limiter::new(100, Duration::from_secs(60));
        let bandwidth_account =
            crate::node::relay::RelayBandwidthAccount::new(100, Duration::from_secs(1));
        Self {
            db,
            config,
            signing_key: None,
            search_cache,
            tier2_limiter,
            bandwidth_account,
        }
    }

    pub fn set_signing_key(&mut self, key: Arc<ed25519_dalek::SigningKey>) {
        self.signing_key = Some(key);
    }

    /// Insert a record with quota check, dedup, and index update.
    /// If a signing key is available, signs the record before storing.
    pub fn insert_record(
        &self,
        record: &mut ContentRecord,
    ) -> Result<records::InsertResult, String> {
        // Sign the record if we have a signing key and it's not already signed
        if let Some(ref sk) = self.signing_key {
            if record.sig.is_empty() {
                let fields = crate::trust::sign::RecordFields {
                    id: record.id.as_bytes().to_vec(),
                    source_url: record.source_url.as_bytes().to_vec(),
                    source_hash: record.source_hash.as_bytes().to_vec(),
                    schema: record.schema.as_bytes().to_vec(),
                    tags: record.tags.join(",").into_bytes(),
                    body: record.body.as_bytes().to_vec(),
                    created_at: record.created_at.to_string().into_bytes(),
                    expires_at: record.expires_at.to_string().into_bytes(),
                    scrape_source: record.scrape_source.as_str().as_bytes().to_vec(),
                    refresh_policy: record.refresh_policy.as_str().as_bytes().to_vec(),
                };
                let sig = crate::trust::sign::sign_record(sk, &fields);
                // Self-verify the signature we just produced
                let vk = sk.verifying_key();
                let sig_bytes = sig.to_bytes();
                let verified = crate::trust::sign::verify_record_sig(&vk, &fields, &sig);
                if !verified {
                    return Err("self-verification of record signature failed".to_string());
                }
                record.sig = sig_bytes.iter().map(|b| format!("{:02x}", b)).collect();
            }
        }

        // Quota check is now inside records::insert_record (within the write transaction)
        let result = records::insert_record(&self.db, record, Some(&self.config))?;

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
    pub fn list_records(
        &self,
        schema: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ContentRecord>, String> {
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

    /// Insert an announcement. If a signing key is available, signs the announcement.
    pub fn insert_announcement(&self, ann: &mut crate::model::Announcement) -> Result<(), String> {
        if let Some(ref sk) = self.signing_key {
            if ann.sig.is_empty() {
                let fields = crate::trust::sign::AnnouncementFields {
                    record_id: ann.record_id.as_bytes().to_vec(),
                    source_hash: ann.source_hash.as_bytes().to_vec(),
                    schema: ann.schema.as_bytes().to_vec(),
                    tags: ann.tags.join(",").into_bytes(),
                    holder_addr: ann.holder_addr.as_bytes().to_vec(),
                    expires_at: ann.expires_at.to_string().into_bytes(),
                };
                let sig = crate::trust::sign::sign_announcement(sk, &fields);
                // Self-verify the announcement signature we just produced
                let vk = sk.verifying_key();
                let verified = crate::trust::sign::verify_announcement_sig(&vk, &fields, &sig);
                if !verified {
                    return Err("self-verification of announcement signature failed".to_string());
                }
                ann.sig = sig
                    .to_bytes()
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect();
            }
        }
        records::insert_announcement(&self.db, ann)
    }

    /// Search the inverted index for records matching a schema and tag filter.
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
    /// Results are cached for repeated queries.
    pub fn search_records(
        &self,
        query_str: &str,
        limit: usize,
    ) -> Result<Vec<ContentRecord>, String> {
        // Build a cache key from the query and limit
        let cache_key = format!("{}:{}", query_str, limit);

        // Check cache first
        if let Some(cached) = self.search_cache.get(&cache_key) {
            if let Ok(records) = serde_json::from_str::<Vec<ContentRecord>>(&cached) {
                return Ok(records);
            }
        }

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
        let results: Vec<ContentRecord> = matched
            .into_iter()
            .take(effective_limit)
            .map(|(r, _)| r)
            .collect();

        // Cache the results
        if let Ok(json) = serde_json::to_string(&results) {
            self.search_cache.insert(&cache_key, &json);
        }

        Ok(results)
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
    pub fn start_expiry_sweeper(
        &self,
        interval: std::time::Duration,
    ) -> tokio::task::JoinHandle<()> {
        expiry::start_expiry_sweeper(self.db.clone(), interval)
    }

    /// Check if a Tier2 announcement request from the given IP is allowed.
    pub fn tier2_allow(&self, ip: &str) -> bool {
        self.tier2_limiter.allow(ip)
    }

    /// Get remaining Tier2 requests for an IP.
    pub fn tier2_remaining(&self, ip: &str) -> u32 {
        self.tier2_limiter.remaining(ip)
    }

    /// Get the number of tracked Tier2 IPs.
    pub fn tier2_len(&self) -> usize {
        self.tier2_limiter.len()
    }

    /// Check if a relay request from the given peer is within bandwidth limits.
    pub fn relay_allow(&self, peer_id: &str, bytes: u64) -> bool {
        self.bandwidth_account.allow(peer_id, bytes)
    }

    /// Record relay bandwidth for a peer without checking the limit.
    pub fn relay_record(&self, peer_id: &str, bytes: u64) {
        self.bandwidth_account.record(peer_id, bytes)
    }

    /// Get remaining relay bandwidth for a peer.
    pub fn relay_remaining(&self, peer_id: &str) -> u64 {
        self.bandwidth_account.remaining(peer_id)
    }

    /// Get the number of tracked relay peers.
    pub fn relay_len(&self) -> usize {
        self.bandwidth_account.len()
    }

    /// Get the number of entries in the search cache.
    pub fn search_cache_len(&self) -> usize {
        self.search_cache.len()
    }

    /// Check if the search cache is empty.
    pub fn search_cache_is_empty(&self) -> bool {
        self.search_cache.is_empty()
    }

    /// Check if no Tier2 IPs are tracked.
    pub fn tier2_is_empty(&self) -> bool {
        self.tier2_limiter.is_empty()
    }

    /// Check if no relay peers are tracked.
    pub fn relay_is_empty(&self) -> bool {
        self.bandwidth_account.is_empty()
    }

    /// Verify a record's signature using the trust::verify module.
    pub fn verify_record(
        &self,
        record: &ContentRecord,
        verifying_key: &ed25519_dalek::VerifyingKey,
    ) -> crate::trust::verify::VerifyResult {
        let sig_bytes: Vec<u8> = (0..record.sig.len())
            .step_by(2)
            .filter_map(|i| {
                u8::from_str_radix(&record.sig[i..i + 2.min(record.sig.len() - i)], 16).ok()
            })
            .collect();
        if sig_bytes.len() != 64 {
            return crate::trust::verify::VerifyResult::fail("invalid signature length");
        }
        let sig_array: [u8; 64] = sig_bytes.try_into().unwrap();
        let sig = ed25519_dalek::Signature::from_bytes(&sig_array);
        let fields = crate::trust::sign::RecordFields {
            id: record.id.as_bytes().to_vec(),
            source_url: record.source_url.as_bytes().to_vec(),
            source_hash: record.source_hash.as_bytes().to_vec(),
            schema: record.schema.as_bytes().to_vec(),
            tags: record.tags.join(",").into_bytes(),
            body: record.body.as_bytes().to_vec(),
            created_at: record.created_at.to_string().into_bytes(),
            expires_at: record.expires_at.to_string().into_bytes(),
            scrape_source: record.scrape_source.as_str().as_bytes().to_vec(),
            refresh_policy: record.refresh_policy.as_str().as_bytes().to_vec(),
        };
        crate::trust::verify::verify_record_signature(verifying_key, &fields, &sig)
    }

    /// Verify an announcement's signature using the trust::verify module.
    pub fn verify_announcement(
        &self,
        ann: &crate::model::Announcement,
        verifying_key: &ed25519_dalek::VerifyingKey,
    ) -> crate::trust::verify::VerifyResult {
        let sig_bytes: Vec<u8> = (0..ann.sig.len())
            .step_by(2)
            .filter_map(|i| u8::from_str_radix(&ann.sig[i..i + 2.min(ann.sig.len() - i)], 16).ok())
            .collect();
        if sig_bytes.len() != 64 {
            return crate::trust::verify::VerifyResult::fail("invalid signature length");
        }
        let sig_array: [u8; 64] = sig_bytes.try_into().unwrap();
        let sig = ed25519_dalek::Signature::from_bytes(&sig_array);
        let fields = crate::trust::sign::AnnouncementFields {
            record_id: ann.record_id.as_bytes().to_vec(),
            source_hash: ann.source_hash.as_bytes().to_vec(),
            schema: ann.schema.as_bytes().to_vec(),
            tags: ann.tags.join(",").into_bytes(),
            holder_addr: ann.holder_addr.as_bytes().to_vec(),
            expires_at: ann.expires_at.to_string().into_bytes(),
        };
        crate::trust::verify::verify_announcement_signature(verifying_key, &fields, &sig)
    }

    /// Verify a record's ID matches the Blake3 hash of its content fields.
    pub fn verify_record_id(&self, record: &ContentRecord) -> crate::trust::verify::VerifyResult {
        crate::trust::verify::verify_record_id(
            &record.id,
            record.source_url.as_bytes(),
            record.source_hash.as_bytes(),
            record.schema.as_bytes(),
            record.tags.join(",").as_bytes(),
            record.body.as_bytes(),
            record.created_at.to_string().as_bytes(),
        )
    }

    /// Check PoW difficulty for a given input.
    pub fn check_pow(
        &self,
        input: &[u8],
        difficulty: u8,
    ) -> Option<crate::trust::pow::PowSolution> {
        crate::trust::pow::mine_pow(input, difficulty)
    }

    /// Verify a PoW solution.
    pub fn verify_pow(&self, input: &[u8], solution: &crate::trust::pow::PowSolution) -> bool {
        crate::trust::pow::verify_pow(input, solution)
    }

    /// Apply a named transform to scraped content.
    pub fn apply_transform(&self, name: &str, input: &str) -> Result<String, String> {
        crate::scraper::sandbox::apply_transform(name, input)
    }

    /// Check storage quota.
    pub fn check_quota(&self, additional_bytes: u64) -> Result<(), String> {
        quota::check_quota(&self.db, &self.config, additional_bytes)
    }

    /// Prune expired Tier2 rate limit buckets.
    pub fn prune_tier2(&self) {
        self.tier2_limiter.prune_expired();
    }

    /// Prune expired relay bandwidth accounts.
    pub fn prune_relay(&self) {
        self.bandwidth_account.prune_expired();
    }

    /// Invalidate a search cache entry.
    pub fn invalidate_search_cache(&self, key: &str) {
        self.search_cache.invalidate(key);
    }

    /// Clear the search cache.
    pub fn clear_search_cache(&self) {
        self.search_cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{schema, RefreshPolicy, ScrapeSource};
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
        let mut r1 = make_record("r1", "sh1", 1000, 2000);
        store.insert_record(&mut r1).unwrap();

        let records = store.list_records(None, 100).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "r1");
    }

    #[test]
    fn store_dedup_keeps_newer() {
        let (_dir, store) = open_test_store();
        let mut older = make_record("r1", "sh1", 1000, 2000);
        let mut newer = make_record("r2", "sh1", 2000, 3000);

        store.insert_record(&mut older).unwrap();
        let result = store.insert_record(&mut newer).unwrap();
        assert!(matches!(result, records::InsertResult::ReplacedNewer));

        let records = store.list_records(None, 100).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "r2");
    }

    #[test]
    fn store_delete_removes_everywhere() {
        let (_dir, store) = open_test_store();
        let mut r1 = make_record("r1", "sh1", 1000, 2000);
        store.insert_record(&mut r1).unwrap();

        assert!(store.delete_record("r1").unwrap());
        assert!(store.get_record("r1").unwrap().is_none());
        assert_eq!(store.list_records(None, 100).unwrap().len(), 0);
    }

    #[test]
    fn store_pin_unpin() {
        let (_dir, store) = open_test_store();
        let mut r1 = make_record("r1", "sh1", 1000, 2000);
        store.insert_record(&mut r1).unwrap();

        assert!(store.pin_record("r1").unwrap());
        assert!(store.is_pinned("r1").unwrap());
        assert!(store.unpin_record("r1").unwrap());
        assert!(!store.is_pinned("r1").unwrap());
    }

    #[test]
    fn store_search_by_tag() {
        let (_dir, store) = open_test_store();
        let mut r1 = make_record("r1", "sh1", 1000, 2000);
        store.insert_record(&mut r1).unwrap();

        let results = store
            .search_index("wiki/article", Some("category"), Some("networking"))
            .unwrap();
        assert!(results.contains(&"r1".to_string()));
    }

    #[test]
    fn store_expiry_sweep() {
        let (_dir, store) = open_test_store();
        let mut r1 = make_record("r1", "sh1", 1000, 100); // expired
        store.insert_record(&mut r1).unwrap();

        let (removed, _) = store.sweep_once().unwrap();
        assert_eq!(removed, 1);
        assert!(store.get_record("r1").unwrap().is_none());
    }

    #[test]
    fn store_search_records() {
        let (_dir, store) = open_test_store();
        let mut r1 = make_record("r1", "sh1", 1000, 2000);
        let mut r2 = make_record("r2", "sh2", 1000, 2000);
        r2.schema = crate::model::schema::RUST_CRATE.to_string();
        r2.body = "Tokio async runtime benchmarks".to_string();

        store.insert_record(&mut r1).unwrap();
        store.insert_record(&mut r2).unwrap();

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

    #[test]
    fn store_tier2_limiter() {
        let (_dir, store) = open_test_store();
        assert!(store.tier2_allow("1.2.3.4"));
        assert!(store.tier2_allow("1.2.3.4"));
    }

    #[test]
    fn store_relay_bandwidth() {
        let (_dir, store) = open_test_store();
        assert!(store.relay_allow("peer1", 1000));
    }

    #[test]
    fn store_apply_transform() {
        let (_dir, store) = open_test_store();
        assert_eq!(
            store.apply_transform("lowercase", "HELLO").unwrap(),
            "hello"
        );
        assert_eq!(
            store.apply_transform("strip_html", "<b>bold</b>").unwrap(),
            "bold"
        );
    }

    #[test]
    fn store_verify_record_id() {
        let (_dir, store) = open_test_store();
        let mut r1 = make_record("r1", "sh1", 1000, 2000);
        // Compute the expected record_id
        let expected_id = crate::trust::sign::compute_record_id(
            r1.source_url.as_bytes(),
            r1.source_hash.as_bytes(),
            r1.schema.as_bytes(),
            r1.tags.join(",").as_bytes(),
            r1.body.as_bytes(),
            r1.created_at.to_string().as_bytes(),
        );
        r1.id = expected_id.clone();
        let result = store.verify_record_id(&r1);
        assert!(
            result.valid,
            "record ID verification failed: {:?}",
            result.reason
        );
    }

    #[test]
    fn store_check_quota() {
        let (_dir, store) = open_test_store();
        // Default config has quota_mb=0, so always allows
        assert!(store.check_quota(1_000_000).is_ok());
    }

    #[test]
    fn store_verify_announcement_with_valid_sig() {
        let (_dir, mut store) = open_test_store();
        let mut rng = rand::rngs::OsRng;
        let sk = ed25519_dalek::SigningKey::generate(&mut rng);
        let vk = sk.verifying_key();
        store.set_signing_key(Arc::new(sk));

        let mut ann = crate::model::Announcement {
            record_id: "r1".to_string(),
            source_hash: "s1".to_string(),
            schema: "kv".to_string(),
            tags: vec![],
            holder_addr: "1.2.3.4:7".to_string(),
            expires_at: 99,
            sig: String::new(),
        };
        store.insert_announcement(&mut ann).unwrap();
        assert!(!ann.sig.is_empty());

        // Verify with the correct key
        let result = store.verify_announcement(&ann, &vk);
        assert!(result.valid, "announcement verification failed: {:?}", result.reason);
    }

    #[test]
    fn store_verify_announcement_with_wrong_key() {
        let (_dir, mut store) = open_test_store();
        let mut rng = rand::rngs::OsRng;
        let sk = ed25519_dalek::SigningKey::generate(&mut rng);
        store.set_signing_key(Arc::new(sk));

        let mut ann = crate::model::Announcement {
            record_id: "r1".to_string(),
            source_hash: "s1".to_string(),
            schema: "kv".to_string(),
            tags: vec![],
            holder_addr: "1.2.3.4:7".to_string(),
            expires_at: 99,
            sig: String::new(),
        };
        store.insert_announcement(&mut ann).unwrap();
        let wrong_sk = ed25519_dalek::SigningKey::generate(&mut rng);
        let wrong_vk = wrong_sk.verifying_key();
        let result = store.verify_announcement(&ann, &wrong_vk);
        assert!(!result.valid);
    }

    #[test]
    fn store_verify_announcement_empty_sig() {
        let (_dir, store) = open_test_store();
        let ann = crate::model::Announcement {
            record_id: "rid1".to_string(),
            source_hash: "sh1".to_string(),
            schema: schema::WIKI_ARTICLE.to_string(),
            tags: vec![],
            holder_addr: "1.2.3.4:7744".to_string(),
            expires_at: 9999,
            sig: String::new(),
        };
        let mut rng = rand::rngs::OsRng;
        let vk = ed25519_dalek::SigningKey::generate(&mut rng).verifying_key();
        let result = store.verify_announcement(&ann, &vk);
        assert!(!result.valid);
        assert!(result.reason.unwrap_or_default().contains("invalid signature length"));
    }

    #[test]
    fn store_verify_record_with_signing_key() {
        let (_dir, mut store) = open_test_store();
        let mut rng = rand::rngs::OsRng;
        let sk = ed25519_dalek::SigningKey::generate(&mut rng);
        let vk = sk.verifying_key();
        store.set_signing_key(Arc::new(sk));

        let mut r1 = make_record("r1", "sh1", 1000, 2000);
        store.insert_record(&mut r1).unwrap();
        assert!(!r1.sig.is_empty());

        // Verify with the correct key
        let result = store.verify_record(&r1, &vk);
        assert!(result.valid, "record verification failed: {:?}", result.reason);
    }

    #[test]
    fn store_verify_record_wrong_key() {
        let (_dir, mut store) = open_test_store();
        let mut rng = rand::rngs::OsRng;
        let sk = ed25519_dalek::SigningKey::generate(&mut rng);
        store.set_signing_key(Arc::new(sk));

        let mut r1 = make_record("r1", "sh1", 1000, 2000);
        store.insert_record(&mut r1).unwrap();

        // Verify with a different key
        let wrong_sk = ed25519_dalek::SigningKey::generate(&mut rng);
        let wrong_vk = wrong_sk.verifying_key();
        let result = store.verify_record(&r1, &wrong_vk);
        assert!(!result.valid);
    }

    #[test]
    fn store_tier2_limiter_methods() {
        let (_dir, store) = open_test_store();
        assert!(store.tier2_is_empty());
        assert_eq!(store.tier2_len(), 0);

        assert!(store.tier2_allow("1.2.3.4"));
        assert!(!store.tier2_is_empty());
        assert_eq!(store.tier2_len(), 1);

        let remaining = store.tier2_remaining("1.2.3.4");
        assert!(remaining > 0);

        let unknown_remaining = store.tier2_remaining("unknown");
        assert_eq!(unknown_remaining, 100); // max_requests from config
    }

    #[test]
    fn store_relay_bandwidth_methods() {
        let (_dir, store) = open_test_store();
        assert!(store.relay_is_empty());
        assert_eq!(store.relay_len(), 0);

        assert!(store.relay_allow("peer1", 1000));
        assert!(!store.relay_is_empty());
        assert_eq!(store.relay_len(), 1);

        let remaining = store.relay_remaining("peer1");
        assert!(remaining > 0);

        store.relay_record("peer1", 500);
        let remaining_after = store.relay_remaining("peer1");
        assert!(remaining_after < remaining);

        let unknown_remaining = store.relay_remaining("unknown");
        assert!(unknown_remaining > 0);
    }

    #[test]
    fn store_search_cache_methods() {
        let (_dir, store) = open_test_store();
        assert!(store.search_cache_is_empty());
        assert_eq!(store.search_cache_len(), 0);

        // Insert a record and search to populate cache
        let mut r1 = make_record("r1", "sh1", 1000, 2000);
        store.insert_record(&mut r1).unwrap();
        let _ = store.search_records("hello", 10);

        assert!(!store.search_cache_is_empty());
        assert!(store.search_cache_len() > 0);

        store.invalidate_search_cache("hello:10");
        store.clear_search_cache();
        assert!(store.search_cache_is_empty());
        assert_eq!(store.search_cache_len(), 0);
    }

    #[test]
    fn store_pow_check_and_verify() {
        let (_dir, store) = open_test_store();
        let difficulty = 8;
        let solution = store.check_pow(b"test-input", difficulty);
        assert!(solution.is_some());
        let sol = solution.unwrap();
        assert!(store.verify_pow(b"test-input", &sol));
        assert!(!store.verify_pow(b"wrong-input", &sol));
    }

    #[test]
    fn store_prune_tier2_and_relay() {
        let (_dir, store) = open_test_store();
        store.tier2_allow("1.2.3.4");
        store.relay_allow("peer1", 100);
        assert!(!store.tier2_is_empty());
        assert!(!store.relay_is_empty());
        // Prune (won't actually remove since windows haven't expired)
        store.prune_tier2();
        store.prune_relay();
        // Still not empty since windows haven't expired
        assert!(!store.tier2_is_empty());
        assert!(!store.relay_is_empty());
    }

    #[test]
    fn store_insert_and_get_announcement() {
        let (_dir, store) = open_test_store();
        let mut ann = crate::model::Announcement {
            record_id: "r1".to_string(),
            source_hash: "s1".to_string(),
            schema: "kv".to_string(),
            tags: vec![],
            holder_addr: "1.2.3.4:7".to_string(),
            expires_at: 99,
            sig: String::new(),
        };
        store.insert_announcement(&mut ann).unwrap();
        // Announcement should be stored (no sig since no signing key)
        assert!(ann.sig.is_empty());
    }
}
