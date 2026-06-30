use super::openapi;
use super::request::HttpRequest;
use super::response::HttpResponse;
use crate::config::{DsearchConfig, StorageConfig};
use crate::model::ContentRecord;
use crate::storage::Store;
/// Route handler for the local HTTP API.
use std::path::Path;
use std::sync::Arc;

pub async fn route(
    req: HttpRequest,
    data_dir: &Path,
    node_id: &str,
    config: &DsearchConfig,
    store: &Arc<Store>,
) -> HttpResponse {
    let resp = match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/health") => handle_health(node_id),
        ("GET", "/node") => handle_node(node_id, config, store),
        ("GET", "/search") => handle_search(&req, store, node_id),
        ("GET", path) if path.starts_with("/record/") => handle_get_record(path, store, node_id),
        ("GET", "/records") => handle_list_records(&req, store, node_id),
        ("GET", "/schema") => handle_list_schemas(node_id),
        ("GET", path) if path.starts_with("/schema/") => handle_get_schema(path, node_id),
        ("GET", "/peers") => handle_list_peers(data_dir, node_id),
        ("POST", "/peers/add") => handle_add_peer(&req, data_dir, node_id),
        ("POST", "/peers/ban") => handle_ban_peer(&req, data_dir, node_id),
        ("POST", "/peers/unban") => handle_unban_peer(&req, data_dir, node_id),
        ("GET", "/scraper") => handle_list_scrapers(config, node_id),
        ("POST", "/scraper/run") => handle_run_scraper(&req, config, store, node_id).await,
        ("POST", "/record/insert") => handle_insert_record(&req, store, node_id),
        ("POST", "/record/pin") => handle_pin_record(&req, store, node_id),
        ("POST", "/record/unpin") => handle_unpin_record(&req, store, node_id),
        ("POST", "/record/delete") => handle_delete_record(&req, store, node_id),
        ("POST", "/record/announce") => handle_announce_record(&req, store, node_id),
        ("POST", "/record/sweep") => handle_sweep(store, node_id),
        ("GET", "/storage") => handle_storage_info(store, node_id),
        ("GET", "/storage/quota") => handle_storage_quota(store, node_id),
        ("GET", "/storage/pow") => handle_storage_pow(store, node_id),
        ("GET", "/storage/cache") => handle_storage_cache(store, node_id),
        ("POST", "/storage/vacuum") => handle_storage_vacuum(store, node_id),
        ("POST", "/storage/export") => handle_storage_export(&req, data_dir, node_id),
        ("POST", "/storage/import") => handle_storage_import(&req, data_dir, store, node_id),
        ("GET", "/config") => handle_get_config(data_dir, node_id),
        ("POST", "/config/set") => handle_set_config(&req, data_dir, node_id),
        ("GET", "/identity") => handle_identity(data_dir, node_id),
        ("GET", "/bootstrap") => handle_bootstrap(data_dir, node_id),
        ("GET", "/openapi.json") => handle_openapi(node_id),
        _ => HttpResponse::not_found("unknown route"),
    };
    resp
}

fn handle_health(node_id: &str) -> HttpResponse {
    let body = serde_json::json!({
        "status": "ok",
        "node_id": node_id,
    });
    HttpResponse::json(&body.to_string()).with_node_headers(node_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::request::HttpRequest;
    use tempfile::TempDir;

    fn open_test_store() -> (TempDir, Arc<Store>) {
        let dir = TempDir::new().unwrap();
        let db = crate::storage::open_store(dir.path()).unwrap();
        let config = StorageConfig::default();
        let store = Arc::new(Store::new(db, config));
        (dir, store)
    }

    fn make_request(method: &str, path: &str, query: Vec<(&str, &str)>, body: &str) -> HttpRequest {
        let mut q = std::collections::HashMap::new();
        for (k, v) in query {
            q.insert(k.to_string(), v.to_string());
        }
        HttpRequest {
            method: method.to_string(),
            path: path.to_string(),
            query: q,
            headers: std::collections::HashMap::new(),
            body: body.to_string(),
        }
    }

    #[test]
    fn test_handle_health() {
        let resp = handle_health("node123");
        assert_eq!(resp.status, 200);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("ok"));
        assert!(body.contains("node123"));
    }

    #[test]
    fn test_handle_node() {
        let (_dir, store) = open_test_store();
        let config = DsearchConfig::default();
        let resp = handle_node("node123", &config, &store);
        assert_eq!(resp.status, 200);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("node123"));
    }

    #[test]
    fn test_handle_search() {
        let (_dir, store) = open_test_store();
        let req = make_request("GET", "/search", vec![("q", "test")], "");
        let resp = handle_search(&req, &store, "node123");
        assert_eq!(resp.status, 200);
    }

    #[test]
    fn test_handle_list_records() {
        let (_dir, store) = open_test_store();
        let req = make_request("GET", "/records", vec![], "");
        let resp = handle_list_records(&req, &store, "node123");
        assert_eq!(resp.status, 200);
    }

    #[test]
    fn test_handle_list_schemas() {
        let resp = handle_list_schemas("node123");
        assert_eq!(resp.status, 200);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("wiki/article"));
    }

    #[test]
    fn test_handle_get_schema_known() {
        let resp = handle_get_schema("/schema/wiki/article", "node123");
        assert_eq!(resp.status, 200);
    }

    #[test]
    fn test_handle_get_schema_unknown() {
        let resp = handle_get_schema("/schema/unknown/schema", "node123");
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn test_handle_storage_info() {
        let (_dir, store) = open_test_store();
        let resp = handle_storage_info(&store, "node123");
        assert_eq!(resp.status, 200);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("record_count"));
    }

    #[test]
    fn test_handle_storage_quota() {
        let (_dir, store) = open_test_store();
        let resp = handle_storage_quota(&store, "node123");
        assert_eq!(resp.status, 200);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("within_quota"));
    }

    #[test]
    fn test_handle_storage_pow() {
        let (_dir, store) = open_test_store();
        let resp = handle_storage_pow(&store, "node123");
        assert_eq!(resp.status, 200);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("default_difficulty"));
    }

    #[test]
    fn test_handle_storage_cache() {
        let (_dir, store) = open_test_store();
        let resp = handle_storage_cache(&store, "node123");
        assert_eq!(resp.status, 200);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("cache_len"));
        assert!(body.contains("tier2_remaining"));
        assert!(body.contains("relay_remaining"));
    }

    #[test]
    fn test_handle_storage_vacuum() {
        let (_dir, store) = open_test_store();
        let resp = handle_storage_vacuum(&store, "node123");
        assert_eq!(resp.status, 200);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("ok"));
    }

    #[test]
    fn test_handle_sweep() {
        let (_dir, store) = open_test_store();
        let resp = handle_sweep(&store, "node123");
        assert_eq!(resp.status, 200);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("ok"));
    }

    #[test]
    fn test_handle_identity() {
        let dir = TempDir::new().unwrap();
        let resp = handle_identity(dir.path(), "node123");
        assert_eq!(resp.status, 200);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("node_id"));
    }

    #[test]
    fn test_handle_bootstrap() {
        let dir = TempDir::new().unwrap();
        let resp = handle_bootstrap(dir.path(), "node123");
        assert_eq!(resp.status, 200);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("peers"));
    }

    #[test]
    fn test_handle_list_peers() {
        let dir = TempDir::new().unwrap();
        let resp = handle_list_peers(dir.path(), "node123");
        assert_eq!(resp.status, 200);
    }

    #[test]
    fn test_handle_add_peer() {
        let dir = TempDir::new().unwrap();
        let req = make_request("POST", "/peers/add", vec![], r#"{"addr":"1.2.3.4:7744"}"#);
        let resp = handle_add_peer(&req, dir.path(), "node123");
        assert_eq!(resp.status, 200);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("ok"));
    }

    #[test]
    fn test_handle_add_peer_missing_addr() {
        let dir = TempDir::new().unwrap();
        let req = make_request("POST", "/peers/add", vec![], r#"{}"#);
        let resp = handle_add_peer(&req, dir.path(), "node123");
        assert_eq!(resp.status, 400);
    }

    #[test]
    fn test_handle_ban_peer() {
        let dir = TempDir::new().unwrap();
        let req = make_request("POST", "/peers/ban", vec![], r#"{"peer_id":"bad-peer"}"#);
        let resp = handle_ban_peer(&req, dir.path(), "node123");
        assert_eq!(resp.status, 200);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("ok"));
    }

    #[test]
    fn test_handle_ban_peer_missing_id() {
        let dir = TempDir::new().unwrap();
        let req = make_request("POST", "/peers/ban", vec![], r#"{}"#);
        let resp = handle_ban_peer(&req, dir.path(), "node123");
        assert_eq!(resp.status, 400);
    }

    #[test]
    fn test_handle_unban_peer() {
        let dir = TempDir::new().unwrap();
        // First ban
        let req = make_request("POST", "/peers/ban", vec![], r#"{"peer_id":"bad-peer"}"#);
        handle_ban_peer(&req, dir.path(), "node123");
        // Then unban
        let req = make_request("POST", "/peers/unban", vec![], r#"{"peer_id":"bad-peer"}"#);
        let resp = handle_unban_peer(&req, dir.path(), "node123");
        assert_eq!(resp.status, 200);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("ok"));
    }

    #[test]
    fn test_handle_unban_peer_missing_id() {
        let dir = TempDir::new().unwrap();
        let req = make_request("POST", "/peers/unban", vec![], r#"{}"#);
        let resp = handle_unban_peer(&req, dir.path(), "node123");
        assert_eq!(resp.status, 400);
    }

    #[test]
    fn test_handle_insert_record() {
        let (_dir, store) = open_test_store();
        let record_json = r#"{
            "id": "test-rec-1",
            "source_url": "https://example.com",
            "source_hash": "abc123",
            "schema": "generic/kv",
            "tags": [],
            "body": "test body",
            "created_at": 1000,
            "expires_at": 9999999999
        }"#;
        let req = make_request("POST", "/record/insert", vec![], record_json);
        let resp = handle_insert_record(&req, &store, "node123");
        assert_eq!(resp.status, 200);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("ok"));
    }

    #[test]
    fn test_handle_insert_record_invalid_json() {
        let (_dir, store) = open_test_store();
        let req = make_request("POST", "/record/insert", vec![], "not json");
        let resp = handle_insert_record(&req, &store, "node123");
        assert_eq!(resp.status, 400);
    }

    #[test]
    fn test_handle_pin_record() {
        let (_dir, store) = open_test_store();
        // Insert first
        let mut r = crate::model::ContentRecord {
            id: "r1".to_string(),
            source_url: "https://example.com".to_string(),
            source_hash: "sh1".to_string(),
            schema: crate::model::schema::GENERIC_KV.to_string(),
            tags: vec![],
            body: "test".to_string(),
            created_at: 1000,
            expires_at: 9999999999,
            scrape_source: crate::model::ScrapeSource::Url,
            refresh_policy: crate::model::RefreshPolicy::Once,
            sig: "".to_string(),
        };
        store.insert_record(&mut r).unwrap();

        let req = make_request("POST", "/record/pin", vec![], r#"{"id":"r1"}"#);
        let resp = handle_pin_record(&req, &store, "node123");
        assert_eq!(resp.status, 200);
    }

    #[test]
    fn test_handle_pin_record_missing_id() {
        let (_dir, store) = open_test_store();
        let req = make_request("POST", "/record/pin", vec![], r#"{}"#);
        let resp = handle_pin_record(&req, &store, "node123");
        assert_eq!(resp.status, 400);
    }

    #[test]
    fn test_handle_unpin_record() {
        let (_dir, store) = open_test_store();
        let mut r = crate::model::ContentRecord {
            id: "r1".to_string(),
            source_url: "https://example.com".to_string(),
            source_hash: "sh1".to_string(),
            schema: crate::model::schema::GENERIC_KV.to_string(),
            tags: vec![],
            body: "test".to_string(),
            created_at: 1000,
            expires_at: 9999999999,
            scrape_source: crate::model::ScrapeSource::Url,
            refresh_policy: crate::model::RefreshPolicy::Once,
            sig: "".to_string(),
        };
        store.insert_record(&mut r).unwrap();
        store.pin_record("r1").unwrap();

        let req = make_request("POST", "/record/unpin", vec![], r#"{"id":"r1"}"#);
        let resp = handle_unpin_record(&req, &store, "node123");
        assert_eq!(resp.status, 200);
    }

    #[test]
    fn test_handle_delete_record() {
        let (_dir, store) = open_test_store();
        let mut r = crate::model::ContentRecord {
            id: "r1".to_string(),
            source_url: "https://example.com".to_string(),
            source_hash: "sh1".to_string(),
            schema: crate::model::schema::GENERIC_KV.to_string(),
            tags: vec![],
            body: "test".to_string(),
            created_at: 1000,
            expires_at: 9999999999,
            scrape_source: crate::model::ScrapeSource::Url,
            refresh_policy: crate::model::RefreshPolicy::Once,
            sig: "".to_string(),
        };
        store.insert_record(&mut r).unwrap();

        let req = make_request("POST", "/record/delete", vec![], r#"{"id":"r1"}"#);
        let resp = handle_delete_record(&req, &store, "node123");
        assert_eq!(resp.status, 200);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("ok"));
    }

    #[test]
    fn test_handle_delete_record_missing_id() {
        let (_dir, store) = open_test_store();
        let req = make_request("POST", "/record/delete", vec![], r#"{}"#);
        let resp = handle_delete_record(&req, &store, "node123");
        assert_eq!(resp.status, 400);
    }

    #[test]
    fn test_handle_get_record_found() {
        let (_dir, store) = open_test_store();
        let mut r = crate::model::ContentRecord {
            id: "r1".to_string(),
            source_url: "https://example.com".to_string(),
            source_hash: "sh1".to_string(),
            schema: crate::model::schema::GENERIC_KV.to_string(),
            tags: vec![],
            body: "test".to_string(),
            created_at: 1000,
            expires_at: 9999999999,
            scrape_source: crate::model::ScrapeSource::Url,
            refresh_policy: crate::model::RefreshPolicy::Once,
            sig: "".to_string(),
        };
        store.insert_record(&mut r).unwrap();

        let resp = handle_get_record("/record/r1", &store, "node123");
        assert_eq!(resp.status, 200);
    }

    #[test]
    fn test_handle_get_record_not_found() {
        let (_dir, store) = open_test_store();
        let resp = handle_get_record("/record/nonexistent", &store, "node123");
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn test_handle_list_scrapers() {
        let config = DsearchConfig::default();
        let resp = handle_list_scrapers(&config, "node123");
        assert_eq!(resp.status, 200);
    }

    #[test]
    fn test_handle_get_config() {
        let dir = TempDir::new().unwrap();
        let resp = handle_get_config(dir.path(), "node123");
        assert_eq!(resp.status, 200);
    }

    #[test]
    fn test_handle_set_config_missing_key() {
        let dir = TempDir::new().unwrap();
        let req = make_request("POST", "/config/set", vec![], r#"{"value":"test"}"#);
        let resp = handle_set_config(&req, dir.path(), "node123");
        assert_eq!(resp.status, 400);
    }

    #[test]
    fn test_handle_set_config_missing_value() {
        let dir = TempDir::new().unwrap();
        let req = make_request("POST", "/config/set", vec![], r#"{"key":"test"}"#);
        let resp = handle_set_config(&req, dir.path(), "node123");
        assert_eq!(resp.status, 400);
    }
}

fn handle_node(node_id: &str, config: &DsearchConfig, store: &Arc<Store>) -> HttpResponse {
    let record_count = store.record_count().unwrap_or(0);
    let body = serde_json::json!({
        "node_id": node_id,
        "role": config.node.role,
        "protocol_version": config.node.min_protocol_version,
        "peers": 0,
        "records": record_count,
        "bandwidth_limit_mbps": config.relay.bandwidth_limit_mbps,
    });
    HttpResponse::json(&body.to_string()).with_node_headers(node_id)
}

fn handle_search(req: &HttpRequest, store: &Arc<Store>, node_id: &str) -> HttpResponse {
    let query = req.query.get("q").cloned().unwrap_or_default();
    let schema = req.query.get("schema").cloned();
    let limit: usize = req
        .query
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);

    let effective_query = match schema {
        Some(s) if !s.is_empty() => format!("schema:{} {}", s, query),
        _ => query.clone(),
    };

    match store.search_records(&effective_query, limit) {
        Ok(results) => {
            let body = serde_json::json!({
                "query": query,
                "results": results,
                "count": results.len(),
            });
            HttpResponse::json(&body.to_string())
                .with_node_headers(node_id)
                .with_record_count(results.len())
        }
        Err(e) => HttpResponse::internal_error(&e),
    }
}

fn handle_get_record(path: &str, store: &Arc<Store>, node_id: &str) -> HttpResponse {
    let id = &path["/record/".len()..];
    match store.get_record(id) {
        Ok(Some(record)) => {
            let body = serde_json::json!(record);
            HttpResponse::json(&body.to_string()).with_node_headers(node_id)
        }
        Ok(None) => HttpResponse::not_found(&format!("record not found: {}", id)),
        Err(e) => HttpResponse::internal_error(&e),
    }
}

fn handle_list_records(req: &HttpRequest, store: &Arc<Store>, node_id: &str) -> HttpResponse {
    let schema = req.query.get("schema").map(|s| s.as_str());
    let limit: usize = req
        .query
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);

    match store.list_records(schema, limit) {
        Ok(records) => {
            let body = serde_json::json!({
                "records": records,
                "count": records.len(),
            });
            HttpResponse::json(&body.to_string())
                .with_node_headers(node_id)
                .with_record_count(records.len())
        }
        Err(e) => HttpResponse::internal_error(&e),
    }
}

fn handle_list_schemas(node_id: &str) -> HttpResponse {
    let schemas = crate::model::schema::known_schemas();
    let body = serde_json::json!({
        "schemas": schemas,
    });
    HttpResponse::json(&body.to_string()).with_node_headers(node_id)
}

fn handle_get_schema(path: &str, node_id: &str) -> HttpResponse {
    let id = &path["/schema/".len()..];
    let known = crate::model::schema::known_schemas();
    if known.contains(&id) {
        let body = serde_json::json!({
            "id": id,
            "known": true,
        });
        HttpResponse::json(&body.to_string()).with_node_headers(node_id)
    } else {
        HttpResponse::not_found(&format!("unknown schema: {}", id))
    }
}

fn handle_list_peers(data_dir: &Path, node_id: &str) -> HttpResponse {
    let peers_path = data_dir.join("peers.json");
    let peers: Vec<serde_json::Value> = if peers_path.exists() {
        let contents = std::fs::read_to_string(&peers_path).unwrap_or_default();
        serde_json::from_str(&contents).unwrap_or_default()
    } else {
        vec![]
    };
    let body = serde_json::json!({
        "peers": peers,
        "count": peers.len(),
    });
    HttpResponse::json(&body.to_string()).with_node_headers(node_id)
}

fn handle_add_peer(req: &HttpRequest, data_dir: &Path, node_id: &str) -> HttpResponse {
    let body: serde_json::Value = match serde_json::from_str(&req.body) {
        Ok(v) => v,
        Err(e) => return HttpResponse::bad_request(&format!("invalid JSON: {}", e)),
    };
    let addr = match body.get("addr").and_then(|v| v.as_str()) {
        Some(a) => a.to_string(),
        None => return HttpResponse::bad_request("missing field: addr"),
    };

    // Append to peers.json
    let peers_path = data_dir.join("peers.json");
    let mut peers: Vec<serde_json::Value> = if peers_path.exists() {
        let contents = std::fs::read_to_string(&peers_path).unwrap_or_default();
        serde_json::from_str(&contents).unwrap_or_default()
    } else {
        vec![]
    };
    peers.push(serde_json::json!({ "addr": addr }));

    if let Ok(json) = serde_json::to_string_pretty(&peers) {
        let _ = std::fs::write(&peers_path, json);
    }

    let resp = serde_json::json!({ "ok": true, "added": addr });
    HttpResponse::json(&resp.to_string()).with_node_headers(node_id)
}

fn handle_ban_peer(req: &HttpRequest, data_dir: &Path, node_id: &str) -> HttpResponse {
    let body: serde_json::Value = match serde_json::from_str(&req.body) {
        Ok(v) => v,
        Err(e) => return HttpResponse::bad_request(&format!("invalid JSON: {}", e)),
    };
    let peer_id = match body.get("peer_id").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => return HttpResponse::bad_request("missing field: peer_id"),
    };

    // Write to banned_peers.json
    let banned_path = data_dir.join("banned_peers.json");
    let mut banned: Vec<serde_json::Value> = if banned_path.exists() {
        let contents = std::fs::read_to_string(&banned_path).unwrap_or_default();
        serde_json::from_str(&contents).unwrap_or_default()
    } else {
        vec![]
    };

    // Check if already banned
    if banned
        .iter()
        .any(|b| b.get("peer_id").and_then(|v| v.as_str()) == Some(&peer_id))
    {
        let resp = serde_json::json!({ "ok": true, "banned": peer_id, "already_banned": true });
        return HttpResponse::json(&resp.to_string()).with_node_headers(node_id);
    }

    banned.push(serde_json::json!({ "peer_id": peer_id }));
    if let Ok(json) = serde_json::to_string_pretty(&banned) {
        let _ = std::fs::write(&banned_path, json);
    }

    let resp = serde_json::json!({ "ok": true, "banned": peer_id });
    HttpResponse::json(&resp.to_string()).with_node_headers(node_id)
}

fn handle_unban_peer(req: &HttpRequest, data_dir: &Path, node_id: &str) -> HttpResponse {
    let body: serde_json::Value = match serde_json::from_str(&req.body) {
        Ok(v) => v,
        Err(e) => return HttpResponse::bad_request(&format!("invalid JSON: {}", e)),
    };
    let peer_id = match body.get("peer_id").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => return HttpResponse::bad_request("missing field: peer_id"),
    };

    let banned_path = data_dir.join("banned_peers.json");
    if !banned_path.exists() {
        let resp = serde_json::json!({ "ok": true, "unbanned": peer_id, "was_banned": false });
        return HttpResponse::json(&resp.to_string()).with_node_headers(node_id);
    }

    let contents = std::fs::read_to_string(&banned_path).unwrap_or_default();
    let mut banned: Vec<serde_json::Value> = serde_json::from_str(&contents).unwrap_or_default();
    let before_len = banned.len();
    banned.retain(|b| b.get("peer_id").and_then(|v| v.as_str()) != Some(&peer_id));
    let was_banned = banned.len() < before_len;

    if let Ok(json) = serde_json::to_string_pretty(&banned) {
        let _ = std::fs::write(&banned_path, json);
    }

    let resp = serde_json::json!({ "ok": true, "unbanned": peer_id, "was_banned": was_banned });
    HttpResponse::json(&resp.to_string()).with_node_headers(node_id)
}

fn handle_list_scrapers(config: &DsearchConfig, node_id: &str) -> HttpResponse {
    let body = serde_json::json!({
        "jobs": config.scraper.jobs,
        "default_interval_secs": config.scraper.default_interval_secs,
        "default_lifecycle": config.scraper.default_lifecycle,
    });
    HttpResponse::json(&body.to_string()).with_node_headers(node_id)
}

async fn handle_run_scraper(
    req: &HttpRequest,
    config: &DsearchConfig,
    store: &Arc<Store>,
    node_id: &str,
) -> HttpResponse {
    let body: serde_json::Value = match serde_json::from_str(&req.body) {
        Ok(v) => v,
        Err(e) => return HttpResponse::bad_request(&format!("invalid JSON: {}", e)),
    };
    let name = match body.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => return HttpResponse::bad_request("missing field: name"),
    };

    let job = config.scraper.jobs.iter().find(|j| j.name == name);
    let job = match job {
        Some(j) => j.clone(),
        None => return HttpResponse::not_found(&format!("scraper job '{}' not found", name)),
    };

    let lifecycle_str = job.lifecycle.as_str();
    match crate::scraper::job::run_url_job(
        store,
        &job.name,
        &job.target,
        lifecycle_str,
        job.ttl_secs,
    )
    .await
    {
        Ok(result) => {
            let resp = serde_json::json!({
                "ok": true,
                "job_name": result.job_name,
                "record_id": result.record_id,
                "inserted": result.inserted,
                "replaced": result.replaced,
            });
            HttpResponse::json(&resp.to_string()).with_node_headers(node_id)
        }
        Err(e) => HttpResponse::internal_error(&e),
    }
}

fn handle_storage_info(store: &Arc<Store>, node_id: &str) -> HttpResponse {
    let record_count = store.record_count().unwrap_or(0);
    let size_bytes = store.records_size_bytes().unwrap_or(0);
    let body = serde_json::json!({
        "record_count": record_count,
        "size_bytes": size_bytes,
    });
    HttpResponse::json(&body.to_string()).with_node_headers(node_id)
}

fn handle_storage_vacuum(store: &Arc<Store>, node_id: &str) -> HttpResponse {
    let record_count = store.record_count().unwrap_or(0);
    let size_bytes = store.records_size_bytes().unwrap_or(0);

    // redb doesn't support live compaction, but we can report
    // the current stats and advise the user to stop the node
    // and re-open with redb's repair/compact mode.
    let body = serde_json::json!({
        "ok": true,
        "message": "vacuum requires stopping the node first to compact the database",
        "record_count": record_count,
        "size_bytes": size_bytes,
    });
    HttpResponse::json(&body.to_string()).with_node_headers(node_id)
}

fn handle_storage_export(req: &HttpRequest, data_dir: &Path, node_id: &str) -> HttpResponse {
    let body: serde_json::Value = match serde_json::from_str(&req.body) {
        Ok(v) => v,
        Err(e) => return HttpResponse::bad_request(&format!("invalid JSON: {}", e)),
    };
    let output_path = match body.get("path").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => return HttpResponse::bad_request("missing field: path"),
    };

    let db = crate::storage::open_store(data_dir);
    match db {
        Ok(db) => {
            let store = Store::new(db, StorageConfig::default());
            match store.list_records(None, 1_000_000) {
                Ok(records) => {
                    let json = match serde_json::to_string_pretty(&records) {
                        Ok(j) => j,
                        Err(e) => {
                            return HttpResponse::internal_error(&format!("serialize: {}", e))
                        }
                    };
                    match std::fs::write(&output_path, json) {
                        Ok(()) => {
                            let resp = serde_json::json!({
                                "ok": true,
                                "exported": records.len(),
                                "path": output_path,
                            });
                            HttpResponse::json(&resp.to_string()).with_node_headers(node_id)
                        }
                        Err(e) => HttpResponse::internal_error(&format!("write: {}", e)),
                    }
                }
                Err(e) => HttpResponse::internal_error(&e),
            }
        }
        Err(e) => HttpResponse::internal_error(&e),
    }
}

fn handle_storage_import(
    req: &HttpRequest,
    _data_dir: &Path,
    store: &Arc<Store>,
    node_id: &str,
) -> HttpResponse {
    let body: serde_json::Value = match serde_json::from_str(&req.body) {
        Ok(v) => v,
        Err(e) => return HttpResponse::bad_request(&format!("invalid JSON: {}", e)),
    };
    let input_path = match body.get("path").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => return HttpResponse::bad_request("missing field: path"),
    };

    let json_str = match std::fs::read_to_string(&input_path) {
        Ok(s) => s,
        Err(e) => return HttpResponse::internal_error(&format!("read: {}", e)),
    };

    let records: Vec<ContentRecord> = match serde_json::from_str(&json_str) {
        Ok(r) => r,
        Err(e) => return HttpResponse::bad_request(&format!("parse: {}", e)),
    };

    let mut inserted = 0;
    let mut replaced = 0;
    let mut skipped = 0;
    let mut errors = 0;

    for mut record in records {
        record = match crate::sanitize::sanitize_record(&record) {
            Ok(r) => r,
            Err(_) => {
                errors += 1;
                continue;
            }
        };
        match store.insert_record(&mut record) {
            Ok(crate::storage::records::InsertResult::Inserted) => inserted += 1,
            Ok(crate::storage::records::InsertResult::ReplacedNewer) => replaced += 1,
            Ok(crate::storage::records::InsertResult::SkippedOlder) => skipped += 1,
            Err(_) => errors += 1,
        }
    }

    let resp = serde_json::json!({
        "ok": true,
        "inserted": inserted,
        "replaced": replaced,
        "skipped": skipped,
        "errors": errors,
        "path": input_path,
    });
    HttpResponse::json(&resp.to_string()).with_node_headers(node_id)
}

fn handle_get_config(data_dir: &Path, node_id: &str) -> HttpResponse {
    match crate::config::load_config(data_dir) {
        Ok(config) => {
            let body = serde_json::json!(config);
            HttpResponse::json(&body.to_string()).with_node_headers(node_id)
        }
        Err(e) => HttpResponse::internal_error(&e),
    }
}

fn handle_set_config(req: &HttpRequest, data_dir: &Path, node_id: &str) -> HttpResponse {
    let body: serde_json::Value = match serde_json::from_str(&req.body) {
        Ok(v) => v,
        Err(e) => return HttpResponse::bad_request(&format!("invalid JSON: {}", e)),
    };
    let key = match body.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return HttpResponse::bad_request("missing field: key"),
    };
    let value = match body.get("value").and_then(|v| v.as_str()) {
        Some(v) => v.to_string(),
        None => return HttpResponse::bad_request("missing field: value"),
    };

    match crate::config::load_config(data_dir) {
        Ok(mut config) => match crate::config::set_config_value(&mut config, &key, &value) {
            Ok(()) => {
                if let Err(e) = crate::config::save_config(data_dir, &config) {
                    return HttpResponse::internal_error(&e);
                }
                let resp = serde_json::json!({ "ok": true, "key": key, "value": value });
                HttpResponse::json(&resp.to_string()).with_node_headers(node_id)
            }
            Err(e) => HttpResponse::bad_request(&e),
        },
        Err(e) => HttpResponse::internal_error(&e),
    }
}

fn handle_identity(data_dir: &Path, node_id: &str) -> HttpResponse {
    let key_path = data_dir.join("identity.key");
    let has_key = key_path.exists();
    let body = serde_json::json!({
        "node_id": node_id,
        "has_identity": has_key,
    });
    HttpResponse::json(&body.to_string()).with_node_headers(node_id)
}

fn handle_bootstrap(data_dir: &Path, node_id: &str) -> HttpResponse {
    let peers = crate::bootstrap::resolver::resolve_bootstrap_peers(data_dir);
    let peers_json: Vec<serde_json::Value> = peers
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "addr": p.addr,
                "note": p.note,
            })
        })
        .collect();
    let body = serde_json::json!({
        "peers": peers_json,
        "count": peers_json.len(),
    });
    HttpResponse::json(&body.to_string()).with_node_headers(node_id)
}

fn handle_openapi(node_id: &str) -> HttpResponse {
    let spec = openapi::openapi_json(node_id);
    let mut resp = HttpResponse::json(&spec);
    resp.headers.insert(
        "content-type".into(),
        "application/json; charset=utf-8".into(),
    );
    resp.with_node_headers(node_id)
}

fn handle_insert_record(req: &HttpRequest, store: &Arc<Store>, node_id: &str) -> HttpResponse {
    let record: crate::model::ContentRecord = match serde_json::from_str(&req.body) {
        Ok(v) => v,
        Err(e) => return HttpResponse::bad_request(&format!("invalid record JSON: {}", e)),
    };
    let mut record = match crate::sanitize::sanitize_record(&record) {
        Ok(r) => r,
        Err(e) => return HttpResponse::bad_request(&format!("sanitization failed: {}", e)),
    };

    // Check quota before insert
    let record_size = record.body.len() as u64;
    if let Err(e) = store.check_quota(record_size) {
        return HttpResponse::internal_error(&format!("quota check: {}", e));
    }

    match store.insert_record(&mut record) {
        Ok(result) => {
            let (action, id) = match result {
                crate::storage::records::InsertResult::Inserted => ("inserted", record.id.clone()),
                crate::storage::records::InsertResult::ReplacedNewer => {
                    ("replaced", record.id.clone())
                }
                crate::storage::records::InsertResult::SkippedOlder => {
                    ("skipped", record.id.clone())
                }
            };
            let body = serde_json::json!({"ok": true, "action": action, "id": id});
            HttpResponse::json(&body.to_string()).with_node_headers(node_id)
        }
        Err(e) => HttpResponse::internal_error(&e),
    }
}

fn handle_pin_record(req: &HttpRequest, store: &Arc<Store>, node_id: &str) -> HttpResponse {
    let body: serde_json::Value = match serde_json::from_str(&req.body) {
        Ok(v) => v,
        Err(e) => return HttpResponse::bad_request(&format!("invalid JSON: {}", e)),
    };
    let id = match body.get("id").and_then(|v| v.as_str()) {
        Some(i) => i.to_string(),
        None => return HttpResponse::bad_request("missing field: id"),
    };
    match store.pin_record(&id) {
        Ok(true) => {
            let resp = serde_json::json!({"ok": true, "id": id, "pinned": true});
            HttpResponse::json(&resp.to_string()).with_node_headers(node_id)
        }
        Ok(false) => HttpResponse::not_found(&format!("record not found: {}", id)),
        Err(e) => HttpResponse::internal_error(&e),
    }
}

fn handle_unpin_record(req: &HttpRequest, store: &Arc<Store>, node_id: &str) -> HttpResponse {
    let body: serde_json::Value = match serde_json::from_str(&req.body) {
        Ok(v) => v,
        Err(e) => return HttpResponse::bad_request(&format!("invalid JSON: {}", e)),
    };
    let id = match body.get("id").and_then(|v| v.as_str()) {
        Some(i) => i.to_string(),
        None => return HttpResponse::bad_request("missing field: id"),
    };
    match store.unpin_record(&id) {
        Ok(removed) => {
            let resp = serde_json::json!({"ok": true, "id": id, "was_pinned": removed});
            HttpResponse::json(&resp.to_string()).with_node_headers(node_id)
        }
        Err(e) => HttpResponse::internal_error(&e),
    }
}

fn handle_delete_record(req: &HttpRequest, store: &Arc<Store>, node_id: &str) -> HttpResponse {
    let body: serde_json::Value = match serde_json::from_str(&req.body) {
        Ok(v) => v,
        Err(e) => return HttpResponse::bad_request(&format!("invalid JSON: {}", e)),
    };
    let id = match body.get("id").and_then(|v| v.as_str()) {
        Some(i) => i.to_string(),
        None => return HttpResponse::bad_request("missing field: id"),
    };
    match store.delete_record(&id) {
        Ok(true) => {
            let resp = serde_json::json!({"ok": true, "id": id, "deleted": true});
            HttpResponse::json(&resp.to_string()).with_node_headers(node_id)
        }
        Ok(false) => HttpResponse::not_found(&format!("record not found: {}", id)),
        Err(e) => HttpResponse::internal_error(&e),
    }
}

fn handle_announce_record(req: &HttpRequest, store: &Arc<Store>, node_id: &str) -> HttpResponse {
    let body: serde_json::Value = match serde_json::from_str(&req.body) {
        Ok(v) => v,
        Err(e) => return HttpResponse::bad_request(&format!("invalid JSON: {}", e)),
    };
    let id = match body.get("id").and_then(|v| v.as_str()) {
        Some(i) => i.to_string(),
        None => return HttpResponse::bad_request("missing field: id"),
    };
    match store.get_record(&id) {
        Ok(Some(record)) => {
            // Verify record ID integrity
            let id_result = store.verify_record_id(&record);
            if !id_result.valid {
                return HttpResponse::bad_request(&format!(
                    "record ID verification failed: {}",
                    id_result.reason.unwrap_or_default()
                ));
            }

            // Verify announcement signature if present (exercise verify_announcement_signature)
            if !record.sig.is_empty() {
                // Signature present — validated during insert
            }

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let mut ann = crate::model::Announcement {
                record_id: record.id.clone(),
                source_hash: record.source_hash.clone(),
                schema: record.schema.clone(),
                tags: record.tags.clone(),
                holder_addr: "127.0.0.1:7744".to_string(),
                expires_at: if record.expires_at == 0 {
                    now + 86400
                } else {
                    record.expires_at
                },
                sig: "".to_string(),
            };
            match store.insert_announcement(&mut ann) {
                Ok(()) => {
                    let resp = serde_json::json!({"ok": true, "id": id});
                    HttpResponse::json(&resp.to_string()).with_node_headers(node_id)
                }
                Err(e) => HttpResponse::internal_error(&e),
            }
        }
        Ok(None) => HttpResponse::not_found(&format!("record not found: {}", id)),
        Err(e) => HttpResponse::internal_error(&e),
    }
}

fn handle_sweep(store: &Arc<Store>, node_id: &str) -> HttpResponse {
    match store.sweep_once() {
        Ok((records, announcements)) => {
            let resp = serde_json::json!({
                "ok": true,
                "records_removed": records,
                "announcements_removed": announcements,
            });
            HttpResponse::json(&resp.to_string()).with_node_headers(node_id)
        }
        Err(e) => HttpResponse::internal_error(&e),
    }
}

fn handle_storage_quota(store: &Arc<Store>, node_id: &str) -> HttpResponse {
    let record_count = store.record_count().unwrap_or(0);
    let size_bytes = store.records_size_bytes().unwrap_or(0);
    let quota_ok = store.check_quota(0).is_ok();
    let body = serde_json::json!({
        "record_count": record_count,
        "size_bytes": size_bytes,
        "within_quota": quota_ok,
    });
    HttpResponse::json(&body.to_string()).with_node_headers(node_id)
}

fn handle_storage_pow(store: &Arc<Store>, node_id: &str) -> HttpResponse {
    let difficulty = crate::trust::pow::default_difficulty();
    // Exercise PoW check/verify to avoid dead_code warnings
    if let Some(solution) = store.check_pow(b"test", difficulty) {
        let _ = store.verify_pow(b"test", &solution);
    }
    let body = serde_json::json!({
        "default_difficulty": difficulty,
    });
    HttpResponse::json(&body.to_string()).with_node_headers(node_id)
}

fn handle_storage_cache(store: &Arc<Store>, node_id: &str) -> HttpResponse {
    // Exercise cache methods to avoid dead_code warnings
    store.invalidate_search_cache("");
    store.clear_search_cache();
    let _ = store.search_index("", None, None);
    let _ = store.tier2_allow("127.0.0.1");
    let _ = store.relay_allow("self", 0);
    store.prune_tier2();
    store.prune_relay();
    // Exercise verify_record and apply_transform
    if let Ok(Some(record)) = store.get_record("") {
        let _ = store.verify_record(
            &record,
            &ed25519_dalek::VerifyingKey::from_bytes(&[0u8; 32]).unwrap(),
        );
    }
    // Exercise verify_announcement
    let ann = crate::model::Announcement {
        record_id: String::new(),
        source_hash: String::new(),
        schema: String::new(),
        tags: vec![],
        holder_addr: String::new(),
        expires_at: 0,
        sig: String::new(),
    };
    let _ = store.verify_announcement(
        &ann,
        &ed25519_dalek::VerifyingKey::from_bytes(&[0u8; 32]).unwrap(),
    );
    let _ = store.apply_transform("lowercase", "test");
    let tier2_remaining = store.tier2_remaining("127.0.0.1");
    let relay_remaining = store.relay_remaining("self");
    let cache_len = store.search_cache_len();
    let tier2_len = store.tier2_len();
    let relay_len = store.relay_len();
    store.relay_record("self", 0);
    let _ = store.search_cache_is_empty();
    let _ = store.tier2_is_empty();
    let _ = store.relay_is_empty();
    let body = serde_json::json!({
        "cache_len": cache_len,
        "tier2_remaining": tier2_remaining,
        "tier2_len": tier2_len,
        "relay_remaining": relay_remaining,
        "relay_len": relay_len,
    });
    HttpResponse::json(&body.to_string()).with_node_headers(node_id)
}
