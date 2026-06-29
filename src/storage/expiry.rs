use redb::Database;
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tracing::{debug, info, warn};

/// Run the expiry sweeper as a background tokio task.
/// Periodically scans for expired records and announcements and removes them.
/// Uses spawn_blocking to avoid blocking the async runtime with redb write transactions.
pub fn start_expiry_sweeper(db: Arc<Database>, interval: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval_timer = time::interval(interval);
        loop {
            interval_timer.tick().await;

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            let db_clone = db.clone();
            tokio::task::spawn_blocking(move || {
                match crate::storage::records::delete_expired_records(&db_clone, now) {
                    Ok(count) if count > 0 => {
                        info!("Expiry sweeper: removed {} expired records", count);
                    }
                    Ok(_) => {
                        debug!("Expiry sweeper: no expired records");
                    }
                    Err(e) => {
                        warn!("Expiry sweeper: error removing expired records: {}", e);
                    }
                }

                match crate::storage::records::delete_expired_announcements(&db_clone, now) {
                    Ok(count) if count > 0 => {
                        info!("Expiry sweeper: removed {} expired announcements", count);
                    }
                    Ok(_) => {
                        debug!("Expiry sweeper: no expired announcements");
                    }
                    Err(e) => {
                        warn!(
                            "Expiry sweeper: error removing expired announcements: {}",
                            e
                        );
                    }
                }
            })
            .await
            .unwrap_or_else(|e| {
                warn!("Expiry sweeper: spawn_blocking task panicked: {}", e);
            });
        }
    })
}

/// Run a single sweep pass (for testing or manual invocation).
pub fn sweep_once(db: &Database) -> Result<(usize, usize), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let records_removed = crate::storage::records::delete_expired_records(db, now)?;
    let announcements_removed = crate::storage::records::delete_expired_announcements(db, now)?;
    Ok((records_removed, announcements_removed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{schema, ContentRecord, RefreshPolicy, ScrapeSource};
    use tempfile::TempDir;

    fn open_test_db() -> (TempDir, Database) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("store.redb");
        let db = Database::builder().create(&path).expect("create db");
        let write_tx = db.begin_write().unwrap();
        write_tx
            .open_table(crate::storage::records::RECORDS_TABLE)
            .unwrap();
        write_tx
            .open_table(crate::storage::records::SOURCE_INDEX_TABLE)
            .unwrap();
        write_tx
            .open_table(crate::storage::records::PINS_TABLE)
            .unwrap();
        write_tx
            .open_table(crate::storage::records::ANNOUNCEMENTS_TABLE)
            .unwrap();
        write_tx.commit().unwrap();
        (dir, db)
    }

    fn make_record(id: &str, source_hash: &str, created_at: u64, expires_at: u64) -> ContentRecord {
        ContentRecord {
            id: id.to_string(),
            source_url: "https://example.com".to_string(),
            source_hash: source_hash.to_string(),
            schema: schema::WIKI_ARTICLE.to_string(),
            tags: vec![],
            body: "Hello world".to_string(),
            created_at,
            expires_at,
            scrape_source: ScrapeSource::Url,
            refresh_policy: RefreshPolicy::Once,
            sig: "".to_string(),
        }
    }

    #[test]
    fn sweep_once_removes_expired() {
        let (_dir, db) = open_test_db();
        let record = make_record("r1", "sh1", 1000, 100); // expires_at in the past
        crate::storage::records::insert_record(&db, &record, None).unwrap();

        let (records, _anns) = sweep_once(&db).unwrap();
        assert_eq!(records, 1);
        assert!(crate::storage::records::get_record(&db, "r1")
            .unwrap()
            .is_none());
    }

    #[test]
    fn sweep_once_keeps_unexpired() {
        let (_dir, db) = open_test_db();
        let record = make_record("r1", "sh1", 1000, 9999999999); // far future
        crate::storage::records::insert_record(&db, &record, None).unwrap();

        let (records, _) = sweep_once(&db).unwrap();
        assert_eq!(records, 0);
        assert!(crate::storage::records::get_record(&db, "r1")
            .unwrap()
            .is_some());
    }
}
