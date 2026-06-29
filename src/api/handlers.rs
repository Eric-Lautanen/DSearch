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

fn handle_node(node_id: &str, config: &DsearchConfig, store: &Arc<Store>) -> HttpResponse {
    let record_count = store.record_count().unwrap_or(0);
    let body = serde_json::json!({
        "node_id": node_id,
        "role": config.node.role,
        "protocol_version": config.node.min_protocol_version,
        "peers": 0,
        "records": record_count,
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
    if banned.iter().any(|b| b.get("peer_id").and_then(|v| v.as_str()) == Some(&peer_id)) {
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
                        Err(e) => return HttpResponse::internal_error(&format!("serialize: {}", e)),
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

fn handle_storage_import(req: &HttpRequest, _data_dir: &Path, store: &Arc<Store>, node_id: &str) -> HttpResponse {
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
            Err(_) => { errors += 1; continue; }
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
