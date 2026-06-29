use super::gateway_keys::GatewayKeyStore;
use super::request::HttpRequest;
use super::response::HttpResponse;
use crate::config::DsearchConfig;
use crate::storage::Store;
/// Gateway API — optional public read-only surface with per-key rate limiting.
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tracing::{error, info, warn};

/// Maximum concurrent gateway connections.
const MAX_CONCURRENT_CONNECTIONS: usize = 100;

/// Start the gateway API server (if enabled in config).
pub async fn start_gateway_server(
    data_dir: std::path::PathBuf,
    config: DsearchConfig,
    store: Arc<Store>,
    node_id: String,
) -> Result<(), String> {
    if !config.gateway.enabled {
        info!("Gateway API disabled in config");
        return Ok(());
    }

    let bind_addr: SocketAddr = config.gateway.bind.parse().map_err(|e| {
        format!(
            "invalid gateway bind address '{}': {}",
            config.gateway.bind, e
        )
    })?;

    let listener = TcpListener::bind(bind_addr)
        .await
        .map_err(|e| format!("gateway bind {}: {}", bind_addr, e))?;
    info!("Gateway API bound to {}", bind_addr);

    let rate_limit = config.gateway.rate_limit_per_min;
    let require_key = config.gateway.require_api_key;
    let key_store = Arc::new(GatewayKeyStore::new(data_dir.clone()));
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));

    let ctx = GatewayContext {
        data_dir,
        node_id,
        store,
        key_store,
        rate_limit,
        require_key,
        semaphore,
    };
    tokio::spawn(gateway_server_loop(listener, ctx));

    Ok(())
}

/// Shared context for gateway server loop and connection handling.
///
/// Groups the many parameters into a single struct to avoid
/// too_many_arguments warnings.
struct GatewayContext {
    data_dir: std::path::PathBuf,
    node_id: String,
    store: Arc<Store>,
    key_store: Arc<GatewayKeyStore>,
    rate_limit: u32,
    require_key: bool,
    semaphore: Arc<Semaphore>,
}

async fn gateway_server_loop(listener: TcpListener, ctx: GatewayContext) {
    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                let ctx = GatewayContext {
                    data_dir: ctx.data_dir.clone(),
                    node_id: ctx.node_id.clone(),
                    store: ctx.store.clone(),
                    key_store: ctx.key_store.clone(),
                    rate_limit: ctx.rate_limit,
                    require_key: ctx.require_key,
                    semaphore: ctx.semaphore.clone(),
                };
                tokio::spawn(async move {
                    let _permit = ctx.semaphore.acquire().await;
                    if let Err(e) = handle_gateway_connection(stream, addr, &ctx).await {
                        warn!("Gateway connection error: {}", e);
                    }
                });
            }
            Err(e) => {
                error!("Gateway accept error: {}", e);
            }
        }
    }
}

async fn handle_gateway_connection(
    mut stream: tokio::net::TcpStream,
    peer_addr: SocketAddr,
    ctx: &GatewayContext,
) -> Result<(), String> {
    let mut buf = vec![0u8; 65536];
    let n = stream
        .read(&mut buf)
        .await
        .map_err(|e| format!("read: {}", e))?;
    if n == 0 {
        return Ok(());
    }

    let raw = String::from_utf8_lossy(&buf[..n]);
    let req = super::request::parse_http_request(&raw)?;

    // Extract API key from Authorization header or query param
    let api_key = extract_api_key(&req);

    // Check rate limit — per-key if authenticated, per-IP if not
    let identifier = if let Some(ref key) = api_key {
        key.clone()
    } else {
        peer_addr.ip().to_string()
    };

    if !ctx.key_store.check_rate_limit(&identifier, ctx.rate_limit) {
        let resp = HttpResponse::new(
            429,
            "Too Many Requests",
            "{\"error\":\"rate limit exceeded\",\"code\":429}",
        );
        let bytes = resp.to_bytes();
        stream
            .write_all(&bytes)
            .await
            .map_err(|e| format!("write: {}", e))?;
        return Ok(());
    }

    // If API key is required and none provided, reject
    if ctx.require_key && api_key.is_none() {
        let resp = HttpResponse::new(
            401,
            "Unauthorized",
            "{\"error\":\"API key required\",\"code\":401}",
        );
        let bytes = resp.to_bytes();
        stream
            .write_all(&bytes)
            .await
            .map_err(|e| format!("write: {}", e))?;
        return Ok(());
    }

    // If API key is provided, validate it
    if let Some(ref key) = api_key {
        if !ctx.key_store.validate_key(key) {
            let resp = HttpResponse::new(
                401,
                "Unauthorized",
                "{\"error\":\"invalid API key\",\"code\":401}",
            );
            let bytes = resp.to_bytes();
            stream
                .write_all(&bytes)
                .await
                .map_err(|e| format!("write: {}", e))?;
            return Ok(());
        }
    }

    // Gateway only allows GET (read-only)
    let resp = if req.method != "GET" {
        HttpResponse::method_not_allowed("gateway is read-only, only GET is allowed")
    } else {
        gateway_route(&req, &ctx.data_dir, &ctx.node_id, &ctx.store)
    };

    let bytes = resp.to_bytes();
    stream
        .write_all(&bytes)
        .await
        .map_err(|e| format!("write: {}", e))?;
    stream.flush().await.map_err(|e| format!("flush: {}", e))?;

    Ok(())
}

/// Gateway routes — subset of local API, read-only.
fn gateway_route(
    req: &HttpRequest,
    _data_dir: &std::path::PathBuf,
    node_id: &str,
    store: &Arc<Store>,
) -> HttpResponse {
    match req.path.as_str() {
        "/health" => {
            let body = serde_json::json!({"status": "ok", "node_id": node_id});
            HttpResponse::json(&body.to_string()).with_node_headers(node_id)
        }
        "/search" => {
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
                    let body = serde_json::json!({"query": query, "results": results, "count": results.len()});
                    HttpResponse::json(&body.to_string())
                        .with_node_headers(node_id)
                        .with_record_count(results.len())
                }
                Err(e) => HttpResponse::internal_error(&e),
            }
        }
        "/records" => {
            let schema = req.query.get("schema").map(|s| s.as_str());
            let limit: usize = req
                .query
                .get("limit")
                .and_then(|v| v.parse().ok())
                .unwrap_or(50);
            match store.list_records(schema, limit) {
                Ok(records) => {
                    let body = serde_json::json!({"records": records, "count": records.len()});
                    HttpResponse::json(&body.to_string())
                        .with_node_headers(node_id)
                        .with_record_count(records.len())
                }
                Err(e) => HttpResponse::internal_error(&e),
            }
        }
        path if path.starts_with("/record/") => {
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
        "/node" => {
            let record_count = store.record_count().unwrap_or(0);
            let body = serde_json::json!({"node_id": node_id, "records": record_count});
            HttpResponse::json(&body.to_string()).with_node_headers(node_id)
        }
        _ => HttpResponse::not_found("unknown route"),
    }
}

fn extract_api_key(req: &HttpRequest) -> Option<String> {
    // Check Authorization: Bearer <key>
    if let Some(auth) = req.headers.get("authorization") {
        if let Some(key) = auth.strip_prefix("Bearer ") {
            return Some(key.trim().to_string());
        }
    }
    // Check ?api_key=<key>
    req.query.get("api_key").cloned()
}
