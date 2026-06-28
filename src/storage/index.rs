#[cfg(test)]
use redb::ReadableTable;
use redb::{Database, TableDefinition};

const INVERTED_INDEX_TABLE: TableDefinition<&str, &str> = TableDefinition::new("inverted_index");

pub fn index_record(
    db: &Database,
    record_id: &str,
    schema: &str,
    tags: &[String],
) -> Result<(), String> {
    for tag in tags {
        let (tag_key, tag_value) = parse_tag(tag);
        let index_key = format!("{}\0{}\0{}", schema, tag_key, tag_value);

        let existing_ids = {
            let read_tx = db
                .begin_read()
                .map_err(|e| format!("index read tx: {}", e))?;
            let table = read_tx
                .open_table(INVERTED_INDEX_TABLE)
                .map_err(|e| format!("open inverted_index: {}", e))?;
            match table.get(&index_key.as_str()) {
                Ok(Some(guard)) => {
                    let current = guard.value().to_string();
                    if current.split(',').any(|id| id == record_id) {
                        continue;
                    }
                    Some(current)
                }
                Ok(None) => None,
                Err(e) => return Err(format!("read inverted_index: {}", e)),
            }
        };

        let ids = match existing_ids {
            Some(current) => format!("{},{}", current, record_id),
            None => record_id.to_string(),
        };

        let write_tx = db
            .begin_write()
            .map_err(|e| format!("index write tx: {}", e))?;
        {
            let mut table = write_tx
                .open_table(INVERTED_INDEX_TABLE)
                .map_err(|e| format!("open inverted_index for write: {}", e))?;
            table
                .insert(index_key.as_str(), ids.as_str())
                .map_err(|e| format!("write inverted_index: {}", e))?;
        }
        write_tx
            .commit()
            .map_err(|e| format!("index commit: {}", e))?;
    }

    Ok(())
}

pub fn deindex_record(
    db: &Database,
    record_id: &str,
    schema: &str,
    tags: &[String],
) -> Result<(), String> {
    for tag in tags {
        let (tag_key, tag_value) = parse_tag(tag);
        let index_key = format!("{}\0{}\0{}", schema, tag_key, tag_value);

        let existing_ids = {
            let read_tx = db
                .begin_read()
                .map_err(|e| format!("deindex read tx: {}", e))?;
            let table = read_tx
                .open_table(INVERTED_INDEX_TABLE)
                .map_err(|e| format!("open inverted_index for deindex read: {}", e))?;
            match table.get(&index_key.as_str()) {
                Ok(Some(guard)) => Some(guard.value().to_string()),
                Ok(None) => None,
                Err(e) => return Err(format!("read inverted_index for deindex: {}", e)),
            }
        };

        if let Some(current) = existing_ids {
            let ids: Vec<&str> = current.split(',').filter(|id| *id != record_id).collect();
            let write_tx = db
                .begin_write()
                .map_err(|e| format!("deindex write tx: {}", e))?;
            {
                let mut table = write_tx
                    .open_table(INVERTED_INDEX_TABLE)
                    .map_err(|e| format!("open inverted_index for deindex write: {}", e))?;
                if ids.is_empty() {
                    table
                        .remove(&index_key.as_str())
                        .map_err(|e| format!("remove inverted_index entry: {}", e))?;
                } else {
                    let new_val = ids.join(",");
                    table
                        .insert(index_key.as_str(), new_val.as_str())
                        .map_err(|e| format!("update inverted_index: {}", e))?;
                }
            }
            write_tx
                .commit()
                .map_err(|e| format!("deindex commit: {}", e))?;
        }
    }

    Ok(())
}

fn parse_tag(tag: &str) -> (&str, &str) {
    match tag.find(':') {
        Some(pos) => (&tag[..pos], &tag[pos + 1..]),
        None => ("", tag),
    }
}

#[cfg(test)]
pub fn search_index(
    db: &Database,
    schema: &str,
    tag_key: Option<&str>,
    tag_value: Option<&str>,
) -> Result<Vec<String>, String> {
    let read_tx = db
        .begin_read()
        .map_err(|e| format!("search read tx: {}", e))?;
    let table = read_tx
        .open_table(INVERTED_INDEX_TABLE)
        .map_err(|e| format!("open inverted_index for search: {}", e))?;

    let mut results = Vec::new();

    let prefix = match (tag_key, tag_value) {
        (Some(tk), Some(tv)) => format!("{}\0{}\0{}", schema, tk, tv),
        (Some(tk), None) => format!("{}\0{}\0", schema, tk),
        (None, None) => format!("{}\0", schema),
        (None, Some(_tv)) => format!("{}\0", schema),
    };

    for entry_result in table.iter().map_err(|e| format!("search iter: {}", e))? {
        let (key_guard, value_guard) =
            entry_result.map_err(|e| format!("search read entry: {}", e))?;
        let key = key_guard.value();

        if !key.starts_with(&prefix) {
            continue;
        }

        let ids = value_guard.value();
        for id in ids.split(',') {
            let id = id.to_string();
            if !id.is_empty() && !results.contains(&id) {
                results.push(id);
            }
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn open_test_db() -> (TempDir, Database) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("store.redb");
        let db = Database::builder().create(&path).expect("create db");
        let write_tx = db.begin_write().unwrap();
        write_tx.open_table(INVERTED_INDEX_TABLE).unwrap();
        write_tx.commit().unwrap();
        (dir, db)
    }

    #[test]
    fn index_and_search_by_schema_and_tag() {
        let (_dir, db) = open_test_db();
        index_record(
            &db,
            "r1",
            "wiki/article",
            &[
                "category:networking".to_string(),
                "level:beginner".to_string(),
            ],
        )
        .unwrap();
        index_record(
            &db,
            "r2",
            "wiki/article",
            &[
                "category:networking".to_string(),
                "level:advanced".to_string(),
            ],
        )
        .unwrap();
        index_record(
            &db,
            "r3",
            "rust/crate",
            &["category:networking".to_string()],
        )
        .unwrap();
    }

    #[test]
    fn deindex_removes_record() {
        let (_dir, db) = open_test_db();
        index_record(
            &db,
            "r1",
            "wiki/article",
            &["category:networking".to_string()],
        )
        .unwrap();
        index_record(
            &db,
            "r2",
            "wiki/article",
            &["category:networking".to_string()],
        )
        .unwrap();

        deindex_record(
            &db,
            "r1",
            "wiki/article",
            &["category:networking".to_string()],
        )
        .unwrap();
    }

    #[test]
    fn parse_tag_with_colon() {
        assert_eq!(parse_tag("category:networking"), ("category", "networking"));
        assert_eq!(parse_tag("networking"), ("", "networking"));
        assert_eq!(parse_tag("a:b:c"), ("a", "b:c"));
    }
}
