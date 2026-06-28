use crate::api::handlers;
use crate::api::request;
use crate::api::response::HttpResponse;
use crate::config::DsearchConfig;
use crate::storage::Store;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{error, info, warn};

/// Start the local HTTP API server.
/// Tries ports from `start_port` up to `start_port + 10`.
/// Writes the actual bound port to `{data_dir}/api.port`.
/// Returns the actual port that was bound.
pub async fn start_api_server(
    data_dir: PathBuf,
    start_port: u16,
    node_id: String,
    config: DsearchConfig,
    store: Arc<Store>,
) -> Result<u16, String> {
    let max_port = start_port + 10;
    let mut bound_port: Option<u16> = None;
    let mut listener: Option<TcpListener> = None;

    for port in start_port..=max_port {
        match TcpListener::bind(format!("127.0.0.1:{}", port)).await {
            Ok(l) => {
                info!("Local API bound to 127.0.0.1:{}", port);
                bound_port = Some(port);
                listener = Some(l);
                break;
            }
            Err(e) => {
                warn!("Port {} unavailable: {}, trying next", port, e);
            }
        }
    }

    let listener = listener.ok_or_else(|| {
        format!(
            "No available port in range {}-{} for local API",
            start_port, max_port
        )
    })?;
    let actual_port = bound_port.unwrap();

    // Write actual port to api.port file
    let port_path = data_dir.join("api.port");
    std::fs::write(&port_path, actual_port.to_string())
        .map_err(|e| format!("write api.port: {}", e))?;

    // Spawn the server loop
    tokio::spawn(api_server_loop(listener, data_dir, node_id, config, store));

    Ok(actual_port)
}

async fn api_server_loop(
    listener: TcpListener,
    data_dir: PathBuf,
    node_id: String,
    config: DsearchConfig,
    store: Arc<Store>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let data_dir = data_dir.clone();
                let node_id = node_id.clone();
                let config = config.clone();
                let store = store.clone();
                tokio::spawn(async move {
                    if let Err(e) =
                        handle_connection(stream, data_dir, node_id, config, store).await
                    {
                        warn!("API connection error: {}", e);
                    }
                });
            }
            Err(e) => {
                error!("API accept error: {}", e);
            }
        }
    }
}

async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    data_dir: PathBuf,
    node_id: String,
    config: DsearchConfig,
    store: Arc<Store>,
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
    let req = request::parse_http_request(&raw)?;

    let resp: HttpResponse = handlers::route(req, &data_dir, &node_id, &config, &store).await;

    let resp_bytes = resp.to_bytes();
    stream
        .write_all(&resp_bytes)
        .await
        .map_err(|e| format!("write: {}", e))?;
    stream.flush().await.map_err(|e| format!("flush: {}", e))?;

    Ok(())
}
