use crate::node::dht::{RoutingEntry, RoutingTable};
use crate::node::roles::NodeRole;
use crate::proto::cert;
use crate::proto::frame::{self, Frame};
use crate::proto::msg_type::*;
use ed25519_dalek::SigningKey;
use quinn::Endpoint;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

pub struct Node {
    pub node_id: String,
    pub signing_key: SigningKey,
    pub role: NodeRole,
    pub data_dir: std::path::PathBuf,
    pub listen_addr: SocketAddr,
    pub routing_table: Arc<RwLock<RoutingTable>>,
    pub endpoint: Option<Endpoint>,
    pub shutdown_tx: Option<mpsc::Sender<()>>,
    pub running: Arc<std::sync::atomic::AtomicBool>,
}

impl Node {
    pub fn new(
        signing_key: SigningKey,
        node_id: String,
        role: NodeRole,
        data_dir: std::path::PathBuf,
        listen_addr: SocketAddr,
    ) -> Self {
        let routing_table = Arc::new(RwLock::new(RoutingTable::new(node_id.clone())));
        Self {
            node_id,
            signing_key,
            role,
            data_dir,
            listen_addr,
            routing_table,
            endpoint: None,
            shutdown_tx: None,
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Start the QUIC endpoint and begin accepting connections.
    pub async fn start(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cert_der = std::fs::read(self.data_dir.join("node.crt"))?;
        let tls_key_der = std::fs::read(self.data_dir.join("identity.tls"))?;

        let server_config = cert::server_config(&cert_der, &tls_key_der)?;
        let client_config = cert::client_config()?;

        let mut endpoint = Endpoint::server(server_config, self.listen_addr)?;
        endpoint.set_default_client_config(client_config);

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);
        self.endpoint = Some(endpoint);
        self.running.store(true, std::sync::atomic::Ordering::SeqCst);

        info!("Node {} listening on {}", &self.node_id[..8], self.listen_addr);

        let routing_table = self.routing_table.clone();
        let node_id = self.node_id.clone();
        let role = self.role.clone();
        let running = self.running.clone();
        let endpoint_ref = self.endpoint.as_ref().unwrap().clone();
        let data_dir = self.data_dir.clone();

        // Main accept loop
        tokio::spawn(async move {
            loop {
                // Periodically write peers to file for CLI access
                {
                    let rt = routing_table.read().await;
                    let peers: Vec<serde_json::Value> = rt.list().iter().map(|e| {
                        serde_json::json!({
                            "node_id": e.node_id,
                            "addr": e.addr,
                            "roles": e.roles,
                            "last_seen": e.last_seen,
                        })
                    }).collect();
                    let peers_json = serde_json::to_string_pretty(&peers).unwrap_or_default();
                    let _ = std::fs::write(data_dir.join("peers.json"), peers_json);
                }

                tokio::select! {
                    incoming = endpoint_ref.accept() => {
                        match incoming {
                            Some(incoming) => {
                                let rt = routing_table.clone();
                                let nid = node_id.clone();
                                let r = role.clone();
                                let running = running.clone();
                                let dd = data_dir.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = handle_connection(incoming, rt, nid, r, running, dd).await {
                                        error!("Connection error: {}", e);
                                    }
                                });
                            }
                            None => break,
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        info!("Shutdown signal received, stopping accept loop");
                        break;
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                        // Periodic peer file update happens above at top of loop
                    }
                }
            }
        });

        Ok(())
    }

    /// Connect to a peer and perform handshake.
    pub async fn connect_to_peer(&self, addr: SocketAddr) -> Result<quinn::Connection, Box<dyn std::error::Error + Send + Sync>> {
        let endpoint = self.endpoint.as_ref().ok_or("Endpoint not started")?;
        let conn = endpoint.connect(addr, "dsearch")?.await?;

        let (mut send, mut recv) = conn.open_bi().await?;

        // Send handshake
        let handshake = Handshake {
            version: PROTOCOL_VERSION,
            node_id: self.node_id.clone(),
            roles: vec![self.role.to_string()],
            capabilities: vec![],
        };
        let frame = Frame::new(MsgType::Handshake, serde_json::to_vec(&handshake)?);
        frame::write_frame(&mut send, &frame).await?;

        // Read HandshakeAck
        if let Some(ack_frame) = frame::read_frame(&mut recv).await? {
            if ack_frame.msg_type == MsgType::HandshakeAck {
                let ack: HandshakeAck = ack_frame.decode_payload()?;
                debug!("HandshakeAck from {} (version {})", &ack.node_id[..8.min(ack.node_id.len())], ack.version);

                if abs_diff(ack.version, PROTOCOL_VERSION) > 1 {
                    warn!("Incompatible protocol version: local={}, remote={}", PROTOCOL_VERSION, ack.version);
                    let goodbye = Goodbye { reason: "incompatible version".to_string() };
                    let goodbye_frame = Frame::new(MsgType::Goodbye, serde_json::to_vec(&goodbye)?);
                    frame::write_frame(&mut send, &goodbye_frame).await?;
                    conn.close(0u32.into(), b"incompatible version");
                    return Err(format!("Incompatible protocol version: {}", ack.version).into());
                }

                let remote_addr = conn.remote_address();
                let entry = RoutingEntry {
                    node_id: ack.node_id.clone(),
                    addr: format!("{}", remote_addr),
                    roles: ack.roles.clone(),
                    last_seen: now_secs(),
                };
                self.routing_table.write().await.insert(entry);
                info!("Peer {} added to routing table", &ack.node_id[..8.min(ack.node_id.len())]);

                // Write peers file immediately
                self.write_peers_file().await;
            }
        }

        Ok(conn)
    }

    /// Graceful shutdown: send Goodbye to all peers, close endpoint.
    pub async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.running.store(false, std::sync::atomic::Ordering::SeqCst);

        // Log how many peers we're disconnecting from
        let peer_count = {
            let rt = self.routing_table.read().await;
            rt.list().len()
        };
        if peer_count > 0 {
            info!("Sending Goodbye to {} peer(s)...", peer_count);
        }

        if let Some(ref endpoint) = self.endpoint {
            endpoint.close(0u32.into(), b"shutdown");
        }

        if let Some(ref tx) = self.shutdown_tx {
            let _ = tx.send(()).await;
        }

        info!("Node {} shut down cleanly", &self.node_id[..8]);
        Ok(())
    }

    /// Write current routing table to peers.json for CLI access.
    async fn write_peers_file(&self) {
        let rt = self.routing_table.read().await;
        let peers: Vec<serde_json::Value> = rt.list().iter().map(|e| {
            serde_json::json!({
                "node_id": e.node_id,
                "addr": e.addr,
                "roles": e.roles,
                "last_seen": e.last_seen,
            })
        }).collect();
        let peers_json = serde_json::to_string_pretty(&peers).unwrap_or_default();
        let _ = std::fs::write(self.data_dir.join("peers.json"), peers_json);
    }
}

async fn handle_connection(
    incoming: quinn::Incoming,
    routing_table: Arc<RwLock<RoutingTable>>,
    local_node_id: String,
    local_role: NodeRole,
    running: Arc<std::sync::atomic::AtomicBool>,
    data_dir: std::path::PathBuf,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let conn = incoming.await?;
    let remote_addr = conn.remote_address();
    debug!("Incoming connection from {}", remote_addr);

    let (mut send, mut recv) = conn.accept_bi().await?;

    // Read handshake
    if let Some(hs_frame) = frame::read_frame(&mut recv).await? {
        if hs_frame.msg_type == MsgType::Handshake {
            let hs: Handshake = hs_frame.decode_payload()?;
            debug!("Handshake from {} (version {})", &hs.node_id[..8.min(hs.node_id.len())], hs.version);

            if abs_diff(hs.version, PROTOCOL_VERSION) > 1 {
                let goodbye = Goodbye { reason: "incompatible version".to_string() };
                let goodbye_frame = Frame::new(MsgType::Goodbye, serde_json::to_vec(&goodbye)?);
                frame::write_frame(&mut send, &goodbye_frame).await?;
                conn.close(0u32.into(), b"incompatible version");
                return Err(format!("Incompatible protocol version: {}", hs.version).into());
            }

            // Send HandshakeAck
            let ack = HandshakeAck {
                version: PROTOCOL_VERSION,
                node_id: local_node_id.clone(),
                roles: vec![local_role.to_string()],
                capabilities: vec![],
            };
            let ack_frame = Frame::new(MsgType::HandshakeAck, serde_json::to_vec(&ack)?);
            frame::write_frame(&mut send, &ack_frame).await?;

            // Add to routing table
            let entry = RoutingEntry {
                node_id: hs.node_id.clone(),
                addr: format!("{}", remote_addr),
                roles: hs.roles.clone(),
                last_seen: now_secs(),
            };
            routing_table.write().await.insert(entry);
            info!("Peer {} connected", &hs.node_id[..8.min(hs.node_id.len())]);

            // Write peers file
            {
                let rt = routing_table.read().await;
                let peers: Vec<serde_json::Value> = rt.list().iter().map(|e| {
                    serde_json::json!({
                        "node_id": e.node_id,
                        "addr": e.addr,
                        "roles": e.roles,
                        "last_seen": e.last_seen,
                    })
                }).collect();
                let peers_json = serde_json::to_string_pretty(&peers).unwrap_or_default();
                let _ = std::fs::write(data_dir.join("peers.json"), peers_json);
            }

            // Handle subsequent messages
            let rt = routing_table.clone();
            let peer_id = hs.node_id.clone();
            let dd = data_dir.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_messages(&mut recv, &mut send, &rt, &peer_id, running, &dd).await {
                    debug!("Message loop ended for {}: {}", &peer_id[..8.min(peer_id.len())], e);
                }
            });
        }
    }

    Ok(())
}

async fn handle_messages(
    recv: &mut quinn::RecvStream,
    send: &mut quinn::SendStream,
    routing_table: &Arc<RwLock<RoutingTable>>,
    peer_node_id: &str,
    running: Arc<std::sync::atomic::AtomicBool>,
    data_dir: &std::path::PathBuf,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    loop {
        if !running.load(std::sync::atomic::Ordering::SeqCst) {
            let goodbye = Goodbye { reason: "shutdown".to_string() };
            let goodbye_frame = Frame::new(MsgType::Goodbye, serde_json::to_vec(&goodbye)?);
            frame::write_frame(send, &goodbye_frame).await?;
            break;
        }

        match frame::read_frame(recv).await? {
            Some(f) => {
                match f.msg_type {
                    MsgType::Ping => {
                        let ping: Ping = f.decode_payload()?;
                        let pong = Pong { nonce: ping.nonce };
                        let pong_frame = Frame::new(MsgType::Pong, serde_json::to_vec(&pong)?);
                        frame::write_frame(send, &pong_frame).await?;
                    }
                    MsgType::FindNode => {
                        let fn_msg: FindNode = f.decode_payload()?;
                        let closest = routing_table.read().await.find_closest(&fn_msg.target_id, 20);
                        let reply = FindNodeReply {
                            nodes: closest.into_iter().map(|e| NodeInfo {
                                id: e.node_id,
                                addr: e.addr,
                            }).collect(),
                        };
                        let reply_frame = Frame::new(MsgType::FindNodeReply, serde_json::to_vec(&reply)?);
                        frame::write_frame(send, &reply_frame).await?;
                    }
                    MsgType::Goodbye => {
                        let gb: Goodbye = f.decode_payload()?;
                        info!("Peer {} sent Goodbye: {}", &peer_node_id[..8.min(peer_node_id.len())], gb.reason);
                        routing_table.write().await.remove(peer_node_id);
                        // Update peers file
                        let rt = routing_table.read().await;
                        let peers: Vec<serde_json::Value> = rt.list().iter().map(|e| {
                            serde_json::json!({
                                "node_id": e.node_id,
                                "addr": e.addr,
                                "roles": e.roles,
                                "last_seen": e.last_seen,
                            })
                        }).collect();
                        let peers_json = serde_json::to_string_pretty(&peers).unwrap_or_default();
                        let _ = std::fs::write(data_dir.join("peers.json"), peers_json);
                        break;
                    }
                    _ => {
                        debug!("Ignoring unknown message type: {:?}", f.msg_type);
                    }
                }
            }
            None => {
                info!("Stream from {} closed", &peer_node_id[..8.min(peer_node_id.len())]);
                routing_table.write().await.remove(peer_node_id);
                let rt = routing_table.read().await;
                let peers: Vec<serde_json::Value> = rt.list().iter().map(|e| {
                    serde_json::json!({
                        "node_id": e.node_id,
                        "addr": e.addr,
                        "roles": e.roles,
                        "last_seen": e.last_seen,
                    })
                }).collect();
                let peers_json = serde_json::to_string_pretty(&peers).unwrap_or_default();
                let _ = std::fs::write(data_dir.join("peers.json"), peers_json);
                break;
            }
        }
    }
    Ok(())
}

fn abs_diff(a: u8, b: u8) -> u8 {
    if a > b { a - b } else { b - a }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
