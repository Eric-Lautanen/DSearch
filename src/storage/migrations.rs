use redb::{Database, TableDefinition};

/// Current storage schema version.
/// Increment when table definitions change.
pub const CURRENT_SCHEMA_VERSION: u64 = 1;

const META_TABLE: TableDefinition<&str, u64> = TableDefinition::new("meta");
const SCHEMA_VERSION_KEY: &str = "schema_version";

/// Run schema version check and migrations on database open.
/// Returns an error if the database is from a future version.
pub fn check_and_migrate(db: &Database) -> Result<(), String> {
    let version = {
        let read_tx = db.begin_read().map_err(|e| format!("migration read tx: {}", e))?;
        let table = read_tx.open_table(META_TABLE);
        match table {
            Ok(t) => t.get(SCHEMA_VERSION_KEY)
                .map_err(|e| format!("read schema_version: {}", e))?
                .map(|v| v.value())
                .unwrap_or(0),
            Err(_) => 0, // fresh database, no meta table yet
        }
    };

    if version > CURRENT_SCHEMA_VERSION {
        return Err(format!(
            "store.redb schema_version {} is from a future version (current: {}). \
             Downgrading is not supported — your data may be corrupted.",
            version, CURRENT_SCHEMA_VERSION
        ));
    }

    if version == CURRENT_SCHEMA_VERSION {
        return Ok(());
    }

    // Run migrations from version -> CURRENT_SCHEMA_VERSION
    let migrator = Migrator::new(version);
    let write_tx = db.begin_write().map_err(|e| format!("migration write tx: {}", e))?;
    migrator.run(db, &write_tx)?;

    // Write the new version
    {
        let mut meta_table = write_tx.open_table(META_TABLE)
            .map_err(|e| format!("open meta table for migration: {}", e))?;
        meta_table.insert(SCHEMA_VERSION_KEY, CURRENT_SCHEMA_VERSION)
            .map_err(|e| format!("write schema_version: {}", e))?;
    }

    write_tx.commit().map_err(|e| format!("migration commit: {}", e))?;
    Ok(())
}

struct Migrator {
    from: u64,
}

impl Migrator {
    fn new(from: u64) -> Self {
        Self { from }
    }

    fn run(&self, _db: &Database, _write_tx: &redb::WriteTransaction) -> Result<(), String> {
        // Future migrations go here:
        // if self.from < 2 { migrate_v1_to_v2(...); }
        // if self.from < 3 { migrate_v2_to_v3(...); }
        // Currently only version 1 exists, so no migrations needed.
        let _ = self.from;
        Ok(())
    }
}

/// Helper to read schema version from a database (for diagnostics).
pub fn get_schema_version(db: &Database) -> Result<u64, String> {
    let read_tx = db.begin_read().map_err(|e| format!("read tx: {}", e))?;
    let table = read_tx.open_table(META_TABLE);
    match table {
        Ok(t) => Ok(t.get(SCHEMA_VERSION_KEY)
            .map_err(|e| format!("read schema_version: {}", e))?
            .map(|v| v.value())
            .unwrap_or(0)),
        Err(_) => Ok(0),
    }
}

/// Open the database at data_dir, check/migrate, and return the schema version.
/// Used by `dsearch doctor` for diagnostics.
pub fn check_and_migrate_on_path(data_dir: &std::path::Path) -> Result<u64, String> {
    let db_path = data_dir.join("store.redb");
    if !db_path.exists() {
        return Ok(0);
    }
    let db = Database::open(&db_path)
        .map_err(|e| format!("open store.redb: {}", e))?;
    check_and_migrate(&db)?;
    get_schema_version(&db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn open_db(dir: &TempDir) -> Database {
        let path = dir.path().join("store.redb");
        Database::builder()
            .create(&path)
            .expect("create db")
    }

    #[test]
    fn fresh_db_gets_schema_version_set() {
        let dir = TempDir::new().unwrap();
        let db = open_db(&dir);
        check_and_migrate(&db).unwrap();
        let v = get_schema_version(&db).unwrap();
        assert_eq!(v, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn future_schema_version_rejected() {
        let dir = TempDir::new().unwrap();
        let db = open_db(&dir);
        // Manually set a future version
        let write_tx = db.begin_write().unwrap();
        {
            let mut meta = write_tx.open_table(META_TABLE).unwrap();
            meta.insert(SCHEMA_VERSION_KEY, 999u64).unwrap();
        }
        write_tx.commit().unwrap();

        let result = check_and_migrate(&db);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("future version"));
    }

    #[test]
    fn same_version_is_noop() {
        let dir = TempDir::new().unwrap();
        let db = open_db(&dir);
        check_and_migrate(&db).unwrap();
        check_and_migrate(&db).unwrap();
        assert_eq!(get_schema_version(&db).unwrap(), CURRENT_SCHEMA_VERSION);
    }
}
