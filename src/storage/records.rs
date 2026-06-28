use crate::model::Announcement;
use crate::model::ContentRecord;
use redb::{Database, ReadableTable, ReadableTableMetadata, TableDefinition};

// Table definitions
pub const RECORDS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("records");
pub const ANNOUNCEMENTS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("announcements");
pub const SOURCE_INDEX_TABLE: TableDefinition<&str, &str> = TableDefinition::new("source_index");
pub const PINS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("pins");

/// Insert a ContentRecord into the store.
pub enum InsertResult {
    Inserted,
    SkippedOlder,
    ReplacedNewer,
}

pub fn insert_record(db: &Database, record: &ContentRecord) -> Result<InsertResult, String> {
    record
        .validate_size()
        .map_err(|e| format!("record validation: {}", e))?;

    // Phase 1: read-only check for dedup
    let dedup_info = {
        let read_tx = db
            .begin_read()
            .map_err(|e| format!("insert read tx: {}", e))?;
        let source_index = read_tx
            .open_table(SOURCE_INDEX_TABLE)
            .map_err(|e| format!("open source_index: {}", e))?;
        match source_index.get(&record.source_hash.as_str()) {
            Ok(Some(existing_id_guard)) => {
                let existing_id = existing_id_guard.value().to_string();
                drop(source_index);
                let records = read_tx
                    .open_table(RECORDS_TABLE)
                    .map_err(|e| format!("open records for dedup check: {}", e))?;
                match records.get(&existing_id.as_str()) {
                    Ok(Some(existing_json_guard)) => {
                        let existing: ContentRecord =
                            serde_json::from_str(existing_json_guard.value())
                                .map_err(|e| format!("parse existing record: {}", e))?;
                        if existing.created_at >= record.created_at {
                            Some((existing_id, false))
                        } else {
                            Some((existing_id, true))
                        }
                    }
                    Ok(None) => None,
                    Err(e) => return Err(format!("read existing record for dedup: {}", e)),
                }
            }
            Ok(None) => None,
            Err(e) => return Err(format!("source_index lookup: {}", e)),
        }
    };

    let result = match dedup_info {
        Some((_existing_id, false)) => return Ok(InsertResult::SkippedOlder),
        Some((existing_id, true)) => {
            let write_tx = db
                .begin_write()
                .map_err(|e| format!("dedup delete write tx: {}", e))?;
            {
                let mut records = write_tx
                    .open_table(RECORDS_TABLE)
                    .map_err(|e| format!("open records for dedup delete: {}", e))?;
                records
                    .remove(&existing_id.as_str())
                    .map_err(|e| format!("remove old record: {}", e))?;
            }
            write_tx
                .commit()
                .map_err(|e| format!("dedup delete commit: {}", e))?;
            InsertResult::ReplacedNewer
        }
        None => InsertResult::Inserted,
    };

    // Phase 2: insert the new record
    let json = serde_json::to_string(record).map_err(|e| format!("serialize record: {}", e))?;

    let write_tx = db
        .begin_write()
        .map_err(|e| format!("insert write tx: {}", e))?;
    {
        let mut records = write_tx
            .open_table(RECORDS_TABLE)
            .map_err(|e| format!("open records: {}", e))?;
        records
            .insert(record.id.as_str(), json.as_str())
            .map_err(|e| format!("insert record: {}", e))?;
    }
    {
        let source_index = write_tx
            .open_table(SOURCE_INDEX_TABLE)
            .map_err(|e| format!("open source_index: {}", e))?;
        // Need to drop the immutable reference before getting a mutable one
        drop(source_index);
        let mut source_index = write_tx
            .open_table(SOURCE_INDEX_TABLE)
            .map_err(|e| format!("reopen source_index: {}", e))?;
        source_index
            .insert(record.source_hash.as_str(), record.id.as_str())
            .map_err(|e| format!("update source_index: {}", e))?;
    }
    write_tx
        .commit()
        .map_err(|e| format!("insert commit: {}", e))?;
    Ok(result)
}

/// Get a ContentRecord by ID.
pub fn get_record(db: &Database, id: &str) -> Result<Option<ContentRecord>, String> {
    let read_tx = db.begin_read().map_err(|e| format!("get read tx: {}", e))?;
    let table = read_tx
        .open_table(RECORDS_TABLE)
        .map_err(|e| format!("open records: {}", e))?;
    match table.get(id) {
        Ok(Some(guard)) => {
            let record: ContentRecord = serde_json::from_str(guard.value())
                .map_err(|e| format!("deserialize record: {}", e))?;
            Ok(Some(record))
        }
        Ok(None) => Ok(None),
        Err(e) => Err(format!("get record: {}", e)),
    }
}

/// List records, optionally filtered by schema, with a limit.
pub fn list_records(
    db: &Database,
    schema: Option<&str>,
    limit: usize,
) -> Result<Vec<ContentRecord>, String> {
    let read_tx = db
        .begin_read()
        .map_err(|e| format!("list read tx: {}", e))?;
    let table = read_tx
        .open_table(RECORDS_TABLE)
        .map_err(|e| format!("open records: {}", e))?;

    let mut results = Vec::new();
    for entry_result in table.iter().map_err(|e| format!("iter scan: {}", e))? {
        let (key_guard, value_guard) = entry_result.map_err(|e| format!("read entry: {}", e))?;
        let json_str = value_guard.value();
        let record: ContentRecord = serde_json::from_str(json_str)
            .map_err(|e| format!("deserialize record {}: {}", key_guard.value(), e))?;

        if let Some(s) = schema {
            if record.schema != s {
                continue;
            }
        }

        results.push(record);
        if results.len() >= limit {
            break;
        }
    }

    Ok(results)
}

/// Delete a record by ID. Also removes from source_index.
pub fn delete_record(db: &Database, id: &str) -> Result<bool, String> {
    // Read the record first to get source_hash
    let source_hash = {
        let read_tx = db
            .begin_read()
            .map_err(|e| format!("delete read tx: {}", e))?;
        let table = read_tx
            .open_table(RECORDS_TABLE)
            .map_err(|e| format!("open records: {}", e))?;
        match table.get(id) {
            Ok(Some(guard)) => {
                let record: ContentRecord = serde_json::from_str(guard.value())
                    .map_err(|e| format!("deserialize record: {}", e))?;
                Some(record.source_hash)
            }
            Ok(None) => return Ok(false),
            Err(e) => return Err(format!("get record for delete: {}", e)),
        }
    };

    let write_tx = db
        .begin_write()
        .map_err(|e| format!("delete write tx: {}", e))?;

    {
        let mut records = write_tx
            .open_table(RECORDS_TABLE)
            .map_err(|e| format!("open records for delete: {}", e))?;
        records
            .remove(id)
            .map_err(|e| format!("remove record: {}", e))?;
    }

    if let Some(sh) = &source_hash {
        // Check if source_index points to this record
        let should_remove = {
            let source_index = write_tx
                .open_table(SOURCE_INDEX_TABLE)
                .map_err(|e| format!("open source_index for delete check: {}", e))?;
            let result = match source_index.get(&sh.as_str()) {
                Ok(Some(existing_id)) => existing_id.value() == id,
                _ => false,
            };
            result
        };
        if should_remove {
            let mut source_index = write_tx
                .open_table(SOURCE_INDEX_TABLE)
                .map_err(|e| format!("reopen source_index for delete: {}", e))?;
            source_index
                .remove(&sh.as_str())
                .map_err(|e| format!("remove source_index: {}", e))?;
        }
    }

    {
        let mut pins = write_tx
            .open_table(PINS_TABLE)
            .map_err(|e| format!("open pins for delete: {}", e))?;
        pins.remove(id).map_err(|e| format!("remove pin: {}", e))?;
    }

    write_tx
        .commit()
        .map_err(|e| format!("delete commit: {}", e))?;
    Ok(true)
}

/// Pin a record by ID.
pub fn pin_record(db: &Database, id: &str) -> Result<bool, String> {
    // Verify record exists
    {
        let read_tx = db.begin_read().map_err(|e| format!("pin read tx: {}", e))?;
        let table = read_tx
            .open_table(RECORDS_TABLE)
            .map_err(|e| format!("open records: {}", e))?;
        if table
            .get(id)
            .map_err(|e| format!("get record: {}", e))?
            .is_none()
        {
            return Ok(false);
        }
    }

    let write_tx = db
        .begin_write()
        .map_err(|e| format!("pin write tx: {}", e))?;
    {
        let mut pins = write_tx
            .open_table(PINS_TABLE)
            .map_err(|e| format!("open pins: {}", e))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let now_str = now.to_string();
        pins.insert(id, now_str.as_str())
            .map_err(|e| format!("insert pin: {}", e))?;
    }
    write_tx
        .commit()
        .map_err(|e| format!("pin commit: {}", e))?;
    Ok(true)
}

/// Unpin a record by ID.
pub fn unpin_record(db: &Database, id: &str) -> Result<bool, String> {
    let write_tx = db
        .begin_write()
        .map_err(|e| format!("unpin write tx: {}", e))?;
    let removed = {
        let mut pins = write_tx
            .open_table(PINS_TABLE)
            .map_err(|e| format!("open pins: {}", e))?;
        let result = match pins.remove(id) {
            Ok(guard) => guard.is_some(),
            Err(e) => return Err(format!("remove pin: {}", e)),
        };
        result
    };
    write_tx
        .commit()
        .map_err(|e| format!("unpin commit: {}", e))?;
    Ok(removed)
}

/// Check if a record is pinned.
pub fn is_pinned(db: &Database, id: &str) -> Result<bool, String> {
    let read_tx = db
        .begin_read()
        .map_err(|e| format!("is_pinned read tx: {}", e))?;
    let table = read_tx
        .open_table(PINS_TABLE)
        .map_err(|e| format!("open pins: {}", e))?;
    match table.get(id) {
        Ok(Some(_)) => Ok(true),
        Ok(None) => Ok(false),
        Err(e) => Err(format!("check pin: {}", e)),
    }
}

/// Insert an Announcement.
pub fn insert_announcement(db: &Database, ann: &Announcement) -> Result<(), String> {
    ann.validate_size()
        .map_err(|e| format!("announcement validation: {}", e))?;

    let key = format!("{}:{}", ann.record_id, ann.holder_addr);
    let json = serde_json::to_string(ann).map_err(|e| format!("serialize announcement: {}", e))?;

    let write_tx = db
        .begin_write()
        .map_err(|e| format!("ann write tx: {}", e))?;
    {
        let mut table = write_tx
            .open_table(ANNOUNCEMENTS_TABLE)
            .map_err(|e| format!("open announcements: {}", e))?;
        table
            .insert(key.as_str(), json.as_str())
            .map_err(|e| format!("insert announcement: {}", e))?;
    }
    write_tx
        .commit()
        .map_err(|e| format!("ann commit: {}", e))?;
    Ok(())
}

/// Delete expired records (where expires_at < now and not pinned).
/// Returns the number of records removed.
pub fn delete_expired_records(db: &Database, now_secs: u64) -> Result<usize, String> {
    // Phase 1: collect expired record IDs and source hashes
    let to_delete: Vec<(String, String)> = {
        let read_tx = db
            .begin_read()
            .map_err(|e| format!("expiry read tx: {}", e))?;
        let records = read_tx
            .open_table(RECORDS_TABLE)
            .map_err(|e| format!("open records for expiry: {}", e))?;
        let pins = read_tx
            .open_table(PINS_TABLE)
            .map_err(|e| format!("open pins for expiry: {}", e))?;

        let mut expired = Vec::new();
        for entry_result in records.iter().map_err(|e| format!("expiry iter: {}", e))? {
            let (key_guard, value_guard) =
                entry_result.map_err(|e| format!("expiry read entry: {}", e))?;
            let key = key_guard.value().to_string();
            let record: ContentRecord = serde_json::from_str(value_guard.value())
                .map_err(|e| format!("deserialize record for expiry: {}", e))?;

            if record.expires_at > 0 && record.expires_at < now_secs {
                let pinned = pins
                    .get(&key.as_str())
                    .map(|v| v.is_some())
                    .unwrap_or(false);
                if !pinned {
                    expired.push((key, record.source_hash));
                }
            }
        }
        expired
    };

    let count = to_delete.len();
    if count == 0 {
        return Ok(0);
    }

    // Phase 2: delete them
    let write_tx = db
        .begin_write()
        .map_err(|e| format!("expiry write tx: {}", e))?;
    for (id, source_hash) in &to_delete {
        {
            let mut records = write_tx
                .open_table(RECORDS_TABLE)
                .map_err(|e| format!("open records for expiry delete: {}", e))?;
            records
                .remove(&id.as_str())
                .map_err(|e| format!("remove expired record: {}", e))?;
        }
        {
            let should_remove = {
                let source_index = write_tx
                    .open_table(SOURCE_INDEX_TABLE)
                    .map_err(|e| format!("open source_index for expiry check: {}", e))?;
                let result = match source_index.get(&source_hash.as_str()) {
                    Ok(Some(existing_id)) => existing_id.value() == id.as_str(),
                    _ => false,
                };
                result
            };
            if should_remove {
                let mut source_index = write_tx
                    .open_table(SOURCE_INDEX_TABLE)
                    .map_err(|e| format!("reopen source_index for expiry delete: {}", e))?;
                source_index
                    .remove(&source_hash.as_str())
                    .map_err(|e| format!("remove source_index for expiry: {}", e))?;
            }
        }
    }
    write_tx
        .commit()
        .map_err(|e| format!("expiry commit: {}", e))?;
    Ok(count)
}

/// Delete expired announcements (where expires_at < now).
pub fn delete_expired_announcements(db: &Database, now_secs: u64) -> Result<usize, String> {
    // Phase 1: collect expired keys
    let to_delete: Vec<String> = {
        let read_tx = db
            .begin_read()
            .map_err(|e| format!("ann expiry read tx: {}", e))?;
        let table = read_tx
            .open_table(ANNOUNCEMENTS_TABLE)
            .map_err(|e| format!("open announcements for expiry: {}", e))?;

        let mut expired = Vec::new();
        for entry_result in table
            .iter()
            .map_err(|e| format!("ann expiry iter: {}", e))?
        {
            let (key_guard, value_guard) =
                entry_result.map_err(|e| format!("ann expiry read entry: {}", e))?;
            let ann: Announcement = serde_json::from_str(value_guard.value())
                .map_err(|e| format!("deserialize announcement for expiry: {}", e))?;

            if ann.expires_at < now_secs {
                expired.push(key_guard.value().to_string());
            }
        }
        expired
    };

    let count = to_delete.len();
    if count == 0 {
        return Ok(0);
    }

    // Phase 2: delete them
    let write_tx = db
        .begin_write()
        .map_err(|e| format!("ann expiry write tx: {}", e))?;
    {
        let mut table = write_tx
            .open_table(ANNOUNCEMENTS_TABLE)
            .map_err(|e| format!("open announcements for expiry delete: {}", e))?;
        for key in &to_delete {
            table
                .remove(&key.as_str())
                .map_err(|e| format!("remove expired announcement: {}", e))?;
        }
    }
    write_tx
        .commit()
        .map_err(|e| format!("ann expiry commit: {}", e))?;
    Ok(count)
}

/// Get the approximate size of the records table in bytes.
pub fn records_size_bytes(db: &Database) -> Result<u64, String> {
    let read_tx = db
        .begin_read()
        .map_err(|e| format!("size read tx: {}", e))?;
    let table = read_tx
        .open_table(RECORDS_TABLE)
        .map_err(|e| format!("open records for size: {}", e))?;

    let mut total: u64 = 0;
    for entry_result in table.iter().map_err(|e| format!("size iter: {}", e))? {
        let (_key_guard, value_guard) =
            entry_result.map_err(|e| format!("size read entry: {}", e))?;
        total += value_guard.value().len() as u64;
    }
    Ok(total)
}

/// Count the number of records.
pub fn record_count(db: &Database) -> Result<u64, String> {
    let read_tx = db
        .begin_read()
        .map_err(|e| format!("count read tx: {}", e))?;
    let table = read_tx
        .open_table(RECORDS_TABLE)
        .map_err(|e| format!("open records for count: {}", e))?;
    table.len().map_err(|e| format!("count: {}", e))
}

#[cfg(test)]
pub fn get_record_id_by_source_hash(
    db: &Database,
    source_hash: &str,
) -> Result<Option<String>, String> {
    let read_tx = db
        .begin_read()
        .map_err(|e| format!("source_index read tx: {}", e))?;
    let table = read_tx
        .open_table(SOURCE_INDEX_TABLE)
        .map_err(|e| format!("open source_index: {}", e))?;
    match table.get(source_hash) {
        Ok(Some(id)) => Ok(Some(id.value().to_string())),
        Ok(None) => Ok(None),
        Err(e) => Err(format!("source_index lookup: {}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{schema, RefreshPolicy, ScrapeSource};
    use tempfile::TempDir;

    fn open_test_db() -> (TempDir, Database) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("store.redb");
        let db = Database::builder().create(&path).expect("create db");
        let write_tx = db.begin_write().unwrap();
        write_tx.open_table(RECORDS_TABLE).unwrap();
        write_tx.open_table(SOURCE_INDEX_TABLE).unwrap();
        write_tx.open_table(PINS_TABLE).unwrap();
        write_tx.open_table(ANNOUNCEMENTS_TABLE).unwrap();
        write_tx.commit().unwrap();
        (dir, db)
    }

    fn make_record(id: &str, source_hash: &str, created_at: u64, expires_at: u64) -> ContentRecord {
        ContentRecord {
            id: id.to_string(),
            source_url: "https://example.com".to_string(),
            source_hash: source_hash.to_string(),
            schema: schema::WIKI_ARTICLE.to_string(),
            tags: vec!["test".to_string()],
            body: "Hello world".to_string(),
            created_at,
            expires_at,
            scrape_source: ScrapeSource::Url,
            refresh_policy: RefreshPolicy::Once,
            sig: "".to_string(),
        }
    }

    #[test]
    fn insert_and_get_record() {
        let (_dir, db) = open_test_db();
        let record = make_record("r1", "sh1", 1000, 2000);
        let result = insert_record(&db, &record).unwrap();
        assert!(matches!(result, InsertResult::Inserted));

        let got = get_record(&db, "r1").unwrap().unwrap();
        assert_eq!(got.id, "r1");
        assert_eq!(got.source_hash, "sh1");
    }

    #[test]
    fn insert_dedup_keeps_newer() {
        let (_dir, db) = open_test_db();
        let older = make_record("r1", "sh1", 1000, 2000);
        let newer = make_record("r2", "sh1", 2000, 3000);

        insert_record(&db, &older).unwrap();
        let result = insert_record(&db, &newer).unwrap();
        assert!(matches!(result, InsertResult::ReplacedNewer));

        assert!(get_record(&db, "r1").unwrap().is_none());
        assert_eq!(get_record(&db, "r2").unwrap().unwrap().id, "r2");
        assert_eq!(
            get_record_id_by_source_hash(&db, "sh1").unwrap(),
            Some("r2".to_string())
        );
    }

    #[test]
    fn insert_dedup_skips_older() {
        let (_dir, db) = open_test_db();
        let newer = make_record("r1", "sh1", 2000, 3000);
        let older = make_record("r2", "sh1", 1000, 2000);

        insert_record(&db, &newer).unwrap();
        let result = insert_record(&db, &older).unwrap();
        assert!(matches!(result, InsertResult::SkippedOlder));

        assert_eq!(get_record(&db, "r1").unwrap().unwrap().id, "r1");
        assert!(get_record(&db, "r2").unwrap().is_none());
        assert_eq!(
            get_record_id_by_source_hash(&db, "sh1").unwrap(),
            Some("r1".to_string())
        );
    }

    #[test]
    fn delete_record_removes_from_source_index() {
        let (_dir, db) = open_test_db();
        let record = make_record("r1", "sh1", 1000, 2000);
        insert_record(&db, &record).unwrap();

        assert!(delete_record(&db, "r1").unwrap());
        assert!(get_record(&db, "r1").unwrap().is_none());
        assert!(get_record_id_by_source_hash(&db, "sh1").unwrap().is_none());
    }

    #[test]
    fn pin_unpin_record() {
        let (_dir, db) = open_test_db();
        let record = make_record("r1", "sh1", 1000, 2000);
        insert_record(&db, &record).unwrap();

        assert!(pin_record(&db, "r1").unwrap());
        assert!(is_pinned(&db, "r1").unwrap());

        assert!(unpin_record(&db, "r1").unwrap());
        assert!(!is_pinned(&db, "r1").unwrap());
    }

    #[test]
    fn pin_nonexistent_record_fails() {
        let (_dir, db) = open_test_db();
        assert!(!pin_record(&db, "nonexistent").unwrap());
    }

    #[test]
    fn list_records_with_schema_filter() {
        let (_dir, db) = open_test_db();
        let r1 = make_record("r1", "sh1", 1000, 2000);
        let mut r2 = make_record("r2", "sh2", 1000, 2000);
        r2.schema = schema::RUST_CRATE.to_string();

        insert_record(&db, &r1).unwrap();
        insert_record(&db, &r2).unwrap();

        let wiki = list_records(&db, Some(schema::WIKI_ARTICLE), 100).unwrap();
        assert_eq!(wiki.len(), 1);
        assert_eq!(wiki[0].id, "r1");

        let rust = list_records(&db, Some(schema::RUST_CRATE), 100).unwrap();
        assert_eq!(rust.len(), 1);
        assert_eq!(rust[0].id, "r2");
    }

    #[test]
    fn expired_records_are_removed() {
        let (_dir, db) = open_test_db();
        let record = make_record("r1", "sh1", 1000, 1500);
        insert_record(&db, &record).unwrap();

        let count = delete_expired_records(&db, 2000).unwrap();
        assert_eq!(count, 1);
        assert!(get_record(&db, "r1").unwrap().is_none());
    }

    #[test]
    fn pinned_expired_records_are_kept() {
        let (_dir, db) = open_test_db();
        let record = make_record("r1", "sh1", 1000, 1500);
        insert_record(&db, &record).unwrap();
        pin_record(&db, "r1").unwrap();

        let count = delete_expired_records(&db, 2000).unwrap();
        assert_eq!(count, 0);
        assert!(get_record(&db, "r1").unwrap().is_some());
    }

    #[test]
    fn record_count_works() {
        let (_dir, db) = open_test_db();
        assert_eq!(record_count(&db).unwrap(), 0);
        insert_record(&db, &make_record("r1", "sh1", 1000, 2000)).unwrap();
        assert_eq!(record_count(&db).unwrap(), 1);
        insert_record(&db, &make_record("r2", "sh2", 1000, 2000)).unwrap();
        assert_eq!(record_count(&db).unwrap(), 2);
    }
}
