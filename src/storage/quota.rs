use redb::{Database, ReadableTable};
use crate::config::StorageConfig;

/// Check if inserting a record would exceed the storage quota.
pub fn check_quota(db: &Database, config: &StorageConfig, additional_bytes: u64) -> Result<(), String> {
    if config.quota_mb == 0 {
        return Ok(());
    }

    let quota_bytes = (config.quota_mb as u64) * 1024 * 1024;
    let current_bytes = crate::storage::records::records_size_bytes(db)?;

    if current_bytes + additional_bytes > quota_bytes {
        match config.quota_action.as_str() {
            "evict_oldest" => {
                let freed = evict_oldest(db, current_bytes + additional_bytes - quota_bytes)?;
                if current_bytes + additional_bytes - freed > quota_bytes {
                    return Err(format!(
                        "Storage quota exceeded ({} MB). Eviction freed {} bytes but still over quota.",
                        config.quota_mb, freed
                    ));
                }
                Ok(())
            }
            "pause_scraper" => {
                Err(format!(
                    "Storage quota exceeded ({} MB). Scraper should pause.",
                    config.quota_mb
                ))
            }
            "warn_only" => {
                tracing::warn!(
                    "Storage quota exceeded ({} MB). warn_only policy — insert allowed.",
                    config.quota_mb
                );
                Ok(())
            }
            _ => {
                Err(format!(
                    "Storage quota exceeded ({} MB). Unknown quota_action: {}",
                    config.quota_mb, config.quota_action
                ))
            }
        }
    } else {
        Ok(())
    }
}

/// Evict oldest ephemeral (non-pinned) records until at least `needed_bytes` are freed.
fn evict_oldest(db: &Database, needed_bytes: u64) -> Result<u64, String> {
    // Phase 1: collect candidates (read-only)
    let mut candidates: Vec<(String, String, u64, u64)> = Vec::new(); // (id, source_hash, created_at, json_len)
    {
        let read_tx = db.begin_read().map_err(|e| format!("evict read tx: {}", e))?;
        let records = read_tx.open_table(crate::storage::records::RECORDS_TABLE)
            .map_err(|e| format!("open records for evict: {}", e))?;
        let pins = read_tx.open_table(crate::storage::records::PINS_TABLE)
            .map_err(|e| format!("open pins for evict: {}", e))?;

        for entry_result in records.iter().map_err(|e| format!("evict iter: {}", e))? {
            let (key_guard, value_guard) = entry_result.map_err(|e| format!("evict read entry: {}", e))?;
            let key = key_guard.value().to_string();
            let json_str = value_guard.value();
            let json_len = json_str.len() as u64;

            let pinned = pins.get(&key.as_str())
                .map(|v| v.is_some())
                .unwrap_or(false);
            if pinned {
                continue;
            }

            let record: crate::model::ContentRecord = serde_json::from_str(json_str)
                .map_err(|e| format!("deserialize record for evict: {}", e))?;
            candidates.push((key, record.source_hash, record.created_at, json_len));
        }
    }

    // Sort by created_at ascending (oldest first)
    candidates.sort_by_key(|c| c.2);

    let mut freed: u64 = 0;
    let mut to_delete: Vec<(String, String)> = Vec::new();

    for (id, source_hash, _created_at, json_len) in candidates {
        if freed >= needed_bytes {
            break;
        }
        freed += json_len;
        to_delete.push((id, source_hash));
    }

    if to_delete.is_empty() {
        return Ok(0);
    }

    // Phase 2: delete the evicted records
    let write_tx = db.begin_write().map_err(|e| format!("evict write tx: {}", e))?;
    for (id, source_hash) in &to_delete {
        {
            let mut records = write_tx.open_table(crate::storage::records::RECORDS_TABLE)
                .map_err(|e| format!("open records for evict delete: {}", e))?;
            records.remove(&id.as_str())
                .map_err(|e| format!("remove evicted record: {}", e))?;
        }
        {
            let should_remove = {
                let source_index = write_tx.open_table(crate::storage::records::SOURCE_INDEX_TABLE)
                    .map_err(|e| format!("open source_index for evict check: {}", e))?;
                let result = match source_index.get(&source_hash.as_str()) {
                    Ok(Some(existing_id)) => existing_id.value() == id.as_str(),
                    _ => false,
                };
                result
            };
            if should_remove {
                let mut source_index = write_tx.open_table(crate::storage::records::SOURCE_INDEX_TABLE)
                    .map_err(|e| format!("reopen source_index for evict delete: {}", e))?;
                source_index.remove(&source_hash.as_str())
                    .map_err(|e| format!("remove source_index for evict: {}", e))?;
            }
        }
    }
    write_tx.commit().map_err(|e| format!("evict commit: {}", e))?;

    tracing::info!("Quota eviction: removed {} records, freed {} bytes", to_delete.len(), freed);
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ContentRecord, ScrapeSource, RefreshPolicy, schema};
    use tempfile::TempDir;

    fn open_test_db() -> (TempDir, Database) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("store.redb");
        let db = Database::builder()
            .create(&path)
            .expect("create db");
        let write_tx = db.begin_write().unwrap();
        write_tx.open_table(crate::storage::records::RECORDS_TABLE).unwrap();
        write_tx.open_table(crate::storage::records::SOURCE_INDEX_TABLE).unwrap();
        write_tx.open_table(crate::storage::records::PINS_TABLE).unwrap();
        write_tx.open_table(crate::storage::records::ANNOUNCEMENTS_TABLE).unwrap();
        write_tx.commit().unwrap();
        (dir, db)
    }

    fn make_record(id: &str, source_hash: &str, created_at: u64) -> ContentRecord {
        ContentRecord {
            id: id.to_string(),
            source_url: "https://example.com".to_string(),
            source_hash: source_hash.to_string(),
            schema: schema::WIKI_ARTICLE.to_string(),
            tags: vec![],
            body: "x".repeat(300_000), // ~300KB body to make quota tests meaningful
            created_at,
            expires_at: 9999999999,
            scrape_source: ScrapeSource::Url,
            refresh_policy: RefreshPolicy::Once,
            sig: "".to_string(),
        }
    }

    #[test]
    fn no_quota_always_allows() {
        let (_dir, db) = open_test_db();
        let config = StorageConfig::default();
        assert!(check_quota(&db, &config, 1_000_000).is_ok());
    }

    #[test]
    fn warn_only_allows_over_quota() {
        let (_dir, db) = open_test_db();
        let config = StorageConfig {
            quota_mb: 1,
            quota_action: "warn_only".to_string(),
            tier2_max_mb: 512,
        };
        for i in 0..5 {
            let r = make_record(&format!("r{}", i), &format!("sh{}", i), 1000 + i);
            crate::storage::records::insert_record(&db, &r).unwrap();
        }
        assert!(check_quota(&db, &config, 1).is_ok());
    }

    #[test]
    fn pause_scraper_rejects_over_quota() {
        let (_dir, db) = open_test_db();
        let config = StorageConfig {
            quota_mb: 1,
            quota_action: "pause_scraper".to_string(),
            tier2_max_mb: 512,
        };
        for i in 0..5 {
            let r = make_record(&format!("r{}", i), &format!("sh{}", i), 1000 + i);
            crate::storage::records::insert_record(&db, &r).unwrap();
        }
        let result = check_quota(&db, &config, 1);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("quota exceeded"));
    }

    #[test]
    fn evict_oldest_frees_space() {
        let (_dir, db) = open_test_db();
        let config = StorageConfig {
            quota_mb: 1,
            quota_action: "evict_oldest".to_string(),
            tier2_max_mb: 512,
        };
        for i in 0..5 {
            let r = make_record(&format!("r{}", i), &format!("sh{}", i), 1000 + i);
            crate::storage::records::insert_record(&db, &r).unwrap();
        }

        let result = check_quota(&db, &config, 1);
        match result {
            Ok(()) => {
                let count = crate::storage::records::record_count(&db).unwrap();
                assert!(count < 5, "expected some records evicted, but count={}", count);
            }
            Err(e) => {
                assert!(e.contains("quota"), "unexpected error: {}", e);
            }
        }
    }
}
