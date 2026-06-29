use crate::node::dht::{RoutingEntry, RoutingTable};
use crate::node::roles::NodeRole;
use crate::proto::cert;
use crate::proto::frame::{self, Frame};
use crate::proto::msg_type::*;
use ed25519_dalek::SigningKey;
use quinn::Endpoint;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{debug, error, info, warn};

/// Default maximum concurrent QUIC connections.
const DEFAULT_MAX_CONNECTIONS: usize = 200;

pub struct Node {
    pub node_id: String,
    pub role: NodeRole,
    pub data_dir: std::path::PathBuf,
    pub listen_addr: SocketAddr,
    pub routing_table: Arc<RwLock<RoutingTable>>,
    pub endpoint: Option<Endpoint>,
    pub shutdown_tx: Option<mpsc::Sender<()>>,
    pub running: Arc<std::sync::atomic::AtomicBool>,
    /// Active connection count, capped at max_connections.
    pub active_connections: Arc<AtomicUsize>,
    /// Maximum concurrent QUIC connections.
    pub max_connections: usize,
    /// Mutex to serialize peers.json writes from concurrent tasks.
    pub peers_file_mutex: Arc<Mutex<()>>,
}

impl Node {
    pub fn new(
        _signing_key: SigningKey,
        node_id: String,
        role: NodeRole,
        data_dir: std::path::PathBuf,
        listen_addr: SocketAddr,
    ) -> Self {
        Self::with_max_connections(
            _signing_key,
            node_id,
            role,
            data_dir,
            listen_addr,
            DEFAULT_MAX_CONNECTIONS,
        )
    }

    pub fn with_max_connections(
        _signing_key: SigningKey,
        node_id: String,
        role: NodeRole,
        data_dir: std::path::PathBuf,
        listen_addr: SocketAddr,
        max_connections: usize,
    ) -> Self {
        let routing_table = Arc::new(RwLock::new(RoutingTable::new(node_id.clone())));
        Self {
            node_id,
            role,
            data_dir,
            listen_addr,
            routing_table,
            endpoint: None,
            shutdown_tx: None,
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            active_connections: Arc::new(AtomicUsize::new(0)),
            max_connections,
            peers_file_mutex: Arc::new(Mutex::new(())),
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
        // Bounded channel for inbound messages — provides backpressure
        // when the node is overwhelmed with incoming data.
        // The inbound_rx is consumed by a dedicated task that processes
        // raw message bytes (e.g. logging, metrics, or forwarding).
        let (inbound_tx, inbound_rx) = mpsc::channel::<Vec<u8>>(256);
        self.shutdown_tx = Some(shutdown_tx);
        self.endpoint = Some(endpoint);
        self.running
            .store(true, std::sync::atomic::Ordering::SeqCst);

        // Spawn a task to drain the inbound channel and log messages
        tokio::spawn(async move {
            let mut rx = inbound_rx;
            while let Some(data) = rx.recv().await {
                tracing::debug!("Inbound message: {} bytes", data.len());
            }
        });

        info!(
            "Node {} listening on {}",
            &self.node_id[..8],
            self.listen_addr
        );

        let routing_table = self.routing_table.clone();
        let node_id = self.node_id.clone();
        let role = self.role;
        let running = self.running.clone();
        let endpoint_ref = self.endpoint.as_ref().unwrap().clone();
        let data_dir = self.data_dir.clone();
        let active_connections = self.active_connections.clone();
        let max_connections = self.max_connections;
        let peers_file_mutex = self.peers_file_mutex.clone();

        // Periodic dead-peer pruning task
        {
            let rt = self.routing_table.clone();
            let running = self.running.clone();
            let dd = self.data_dir.clone();
            let pfm = self.peers_file_mutex.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                loop {
                    interval.tick().await;
                    if !running.load(std::sync::atomic::Ordering::SeqCst) {
                        break;
                    }
                    let pruned = rt.write().await.prune_dead_peers();
                    if pruned > 0 {
                        info!("Pruned {} stale peers from routing table", pruned);
                        write_peers_file(&rt, &dd, &pfm).await;
                    }
                }
            });
        }

        // Main accept loop
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    incoming = endpoint_ref.accept() => {
                        match incoming {
                            Some(incoming) => {
                                // Enforce connection pool cap
                                if active_connections.load(Ordering::Relaxed) >= max_connections {
                                    warn!("Connection pool full ({}/{}), rejecting incoming", active_connections.load(Ordering::Relaxed), max_connections);
                                    // Accept then immediately close to avoid hanging the remote
                                    if let Ok(conn) = incoming.await { conn.close(0u32.into(), b"connection pool full"); }
                                    continue;
                                }
                                let rt = routing_table.clone();
                                let nid = node_id.clone();
                                let r = role;
                                let running = running.clone();
                                let dd = data_dir.clone();
                                let ac = active_connections.clone();
                                let ib_tx = inbound_tx.clone();
                                let pfm = peers_file_mutex.clone();
                                tokio::spawn(async move {
                                    ac.fetch_add(1, Ordering::Relaxed);
                                    let result = handle_connection(incoming, rt, nid, r, running, dd, ib_tx, pfm).await;
                                    ac.fetch_sub(1, Ordering::Relaxed);
                                    if let Err(e) = result {
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
                }
            }
        });

        Ok(())
    }

    /// Connect to a peer and perform handshake.
    /// Spawns a persistent message-handling loop on the outbound stream
    /// so the stream stays alive (not dropped after handshake).
    pub async fn connect_to_peer(
        &self,
        addr: SocketAddr,
    ) -> Result<quinn::Connection, Box<dyn std::error::Error + Send + Sync>> {
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
        let mut peer_node_id = String::new();
        if let Some(ack_frame) = frame::read_frame(&mut recv).await? {
            if ack_frame.msg_type == MsgType::HandshakeAck {
                let ack: HandshakeAck = ack_frame.decode_payload()?;
                debug!(
                    "HandshakeAck from {} (version {})",
                    &ack.node_id[..8.min(ack.node_id.len())],
                    ack.version
                );

                if abs_diff(ack.version, PROTOCOL_VERSION) > 1 {
                    warn!(
                        "Incompatible protocol version: local={}, remote={}",
                        PROTOCOL_VERSION, ack.version
                    );
                    let goodbye = Goodbye {
                        reason: "incompatible version".to_string(),
                    };
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
                peer_node_id = ack.node_id.clone();
                self.routing_table.write().await.insert(entry);
                info!(
                    "Peer {} added to routing table (outbound)",
                    &ack.node_id[..8.min(ack.node_id.len())]
                );

                // Write peers file immediately
                {
                    let _lock = self.peers_file_mutex.lock().await;
                    self.write_peers_file_inner().await;
                }
            }
        }

        // Keep the outbound stream alive by spawning a persistent message loop,
        // exactly like the inbound path does. Without this, send/recv drop
        // when connect_to_peer returns, closing the stream and causing the
        // remote side to immediately remove us from its routing table.
        let routing_table = self.routing_table.clone();
        let running = self.running.clone();
        let data_dir = self.data_dir.clone();
        let peer_id = peer_node_id.clone();
        let pfm = self.peers_file_mutex.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_messages(
                &mut recv,
                &mut send,
                &routing_table,
                &peer_id,
                running,
                &data_dir,
                &pfm,
            )
            .await
            {
                debug!(
                    "Outbound message loop ended for {}: {}",
                    &peer_id[..8.min(peer_id.len())],
                    e
                );
            }
        });

        Ok(conn)
    }

    /// Graceful shutdown: set running=false so handle_messages loops send
    /// Goodbye, then close the endpoint (which tears down remaining streams).
    pub async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let peer_count = {
            let rt = self.routing_table.read().await;
            rt.list().len()
        };
        if peer_count > 0 {
            info!("Shutting down with {} peer(s) connected", peer_count);
        }

        // Signal all message loops to send Goodbye and exit.
        // Must happen *before* we close the endpoint, or the streams
        // are torn down before the Goodbye can be written.
        self.running
            .store(false, std::sync::atomic::Ordering::SeqCst);

        // Give the message loops a moment to send their Goodbye frames
        if peer_count > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
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
    /// Callers should hold peers_file_mutex before calling this.
    async fn write_peers_file_inner(&self) {
        let rt = self.routing_table.read().await;
        let peers: Vec<serde_json::Value> = rt
            .list()
            .iter()
            .map(|e| {
                serde_json::json!({
                    "node_id": e.node_id,
                    "addr": e.addr,
                    "roles": e.roles,
                    "last_seen": e.last_seen,
                })
            })
            .collect();
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
    _inbound_tx: mpsc::Sender<Vec<u8>>,
    peers_file_mutex: Arc<Mutex<()>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let conn = incoming.await?;
    let remote_addr = conn.remote_address();
    debug!("Incoming connection from {}", remote_addr);

    let (mut send, mut recv) = conn.accept_bi().await?;

    // Read handshake
    if let Some(hs_frame) = frame::read_frame(&mut recv).await? {
        if hs_frame.msg_type == MsgType::Handshake {
            let hs: Handshake = hs_frame.decode_payload()?;
            debug!(
                "Handshake from {} (version {})",
                &hs.node_id[..8.min(hs.node_id.len())],
                hs.version
            );

            if abs_diff(hs.version, PROTOCOL_VERSION) > 1 {
                let goodbye = Goodbye {
                    reason: "incompatible version".to_string(),
                };
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
            info!(
                "Peer {} connected (inbound, added to routing table)",
                &hs.node_id[..8.min(hs.node_id.len())]
            );

            // Write peers file immediately after inbound insert
            write_peers_file(&routing_table, &data_dir, &peers_file_mutex).await;
            info!("Wrote peers.json after inbound connect");

            // Handle subsequent messages
            let rt = routing_table.clone();
            let peer_id = hs.node_id.clone();
            let dd = data_dir.clone();
            let pfm = peers_file_mutex.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    handle_messages(&mut recv, &mut send, &rt, &peer_id, running, &dd, &pfm).await
                {
                    debug!(
                        "Message loop ended for {}: {}",
                        &peer_id[..8.min(peer_id.len())],
                        e
                    );
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
    peers_file_mutex: &Arc<Mutex<()>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Cleanup runs on every exit path — clean close, explicit Goodbye,
    // or connection error. Without this, a hard connection close (e.g.
    // endpoint.close()) returns Err from read_frame, and the ? operator
    // would skip the cleanup that only the None arm did.
    let result =
        handle_messages_inner(recv, send, routing_table, peer_node_id, running, data_dir).await;

    // Always remove peer and update peers.json, regardless of exit reason
    routing_table.write().await.remove(peer_node_id);
    write_peers_file(routing_table, data_dir, peers_file_mutex).await;

    match &result {
        Ok(()) => info!(
            "Peer {} disconnected (clean)",
            &peer_node_id[..8.min(peer_node_id.len())]
        ),
        Err(e) => info!(
            "Peer {} disconnected (error: {})",
            &peer_node_id[..8.min(peer_node_id.len())],
            e
        ),
    }

    result
}

async fn handle_messages_inner(
    recv: &mut quinn::RecvStream,
    send: &mut quinn::SendStream,
    routing_table: &Arc<RwLock<RoutingTable>>,
    peer_node_id: &str,
    running: Arc<std::sync::atomic::AtomicBool>,
    _data_dir: &std::path::PathBuf,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    loop {
        let frame_result = tokio::select! {
            f = frame::read_frame(recv) => f,
            _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                if !running.load(std::sync::atomic::Ordering::SeqCst) {
                    let goodbye = Goodbye { reason: "shutdown".to_string() };
                    let goodbye_frame = Frame::new(MsgType::Goodbye, serde_json::to_vec(&goodbye)?);
                    frame::write_frame(send, &goodbye_frame).await?;
                    info!("Sent Goodbye to {} (shutdown)", &peer_node_id[..8.min(peer_node_id.len())]);
                    return Ok(());
                }
                continue;
            }
        };

        match frame_result? {
            Some(f) => match f.msg_type {
                MsgType::Ping => {
                    let ping: Ping = f.decode_payload()?;
                    let pong = Pong { nonce: ping.nonce };
                    let pong_frame = Frame::new(MsgType::Pong, serde_json::to_vec(&pong)?);
                    frame::write_frame(send, &pong_frame).await?;
                }
                MsgType::FindNode => {
                    let fn_msg: FindNode = f.decode_payload()?;
                    let closest = routing_table
                        .read()
                        .await
                        .find_closest(&fn_msg.target_id, 20);
                    let reply = FindNodeReply {
                        nodes: closest
                            .into_iter()
                            .map(|e| NodeInfo {
                                id: e.node_id,
                                addr: e.addr,
                            })
                            .collect(),
                    };
                    let reply_frame =
                        Frame::new(MsgType::FindNodeReply, serde_json::to_vec(&reply)?);
                    frame::write_frame(send, &reply_frame).await?;
                }
                MsgType::Announce => {
                    let ann: Announce = f.decode_payload()?;
                    debug!(
                        "Announce from {}: record_id={} holder={}",
                        &peer_node_id[..8.min(peer_node_id.len())],
                        ann.record_id,
                        ann.holder_addr
                    );
                    let ack = AnnounceAck {
                        record_id: ann.record_id.clone(),
                        accepted: true,
                        reason: String::new(),
                    };
                    let ack_frame = Frame::new(MsgType::AnnounceAck, serde_json::to_vec(&ack)?);
                    frame::write_frame(send, &ack_frame).await?;
                }
                MsgType::AnnounceAck => {
                    let ack: AnnounceAck = f.decode_payload()?;
                    debug!(
                        "AnnounceAck from {}: record_id={} accepted={}",
                        &peer_node_id[..8.min(peer_node_id.len())],
                        ack.record_id,
                        ack.accepted
                    );
                }
                MsgType::SearchQuery => {
                    let sq: SearchQuery = f.decode_payload()?;
                    debug!(
                        "SearchQuery from {}: query={:?}",
                        &peer_node_id[..8.min(peer_node_id.len())],
                        sq.query
                    );
                    // Respond with empty results — actual search requires store access
                    // which is wired through the API layer. The DHT fan-out path
                    // will forward queries to closer peers.
                    let reply = SearchReply {
                        query: sq.query.clone(),
                        results: vec![],
                        from_node: peer_node_id.to_string(),
                    };
                    let reply_frame =
                        Frame::new(MsgType::SearchReply, serde_json::to_vec(&reply)?);
                    frame::write_frame(send, &reply_frame).await?;
                }
                MsgType::SearchReply => {
                    let sr: SearchReply = f.decode_payload()?;
                    debug!(
                        "SearchReply from {}: {} results",
                        &peer_node_id[..8.min(peer_node_id.len())],
                        sr.results.len()
                    );
                }
                MsgType::RecordFetch => {
                    let rf: RecordFetch = f.decode_payload()?;
                    debug!(
                        "RecordFetch from {}: record_id={}",
                        &peer_node_id[..8.min(peer_node_id.len())],
                        rf.record_id
                    );
                    // Respond with not_found — actual record lookup requires store access
                    let reply = RecordReply {
                        record_id: rf.record_id.clone(),
                        record_json: None,
                        not_found: true,
                    };
                    let reply_frame =
                        Frame::new(MsgType::RecordReply, serde_json::to_vec(&reply)?);
                    frame::write_frame(send, &reply_frame).await?;
                }
                MsgType::RecordReply => {
                    let rr: RecordReply = f.decode_payload()?;
                    debug!(
                        "RecordReply from {}: record_id={} found={}",
                        &peer_node_id[..8.min(peer_node_id.len())],
                        rr.record_id,
                        !rr.not_found
                    );
                }
                MsgType::ReplicatePush => {
                    let rp: ReplicatePush = f.decode_payload()?;
                    debug!(
                        "ReplicatePush from {}: record_id={}",
                        &peer_node_id[..8.min(peer_node_id.len())],
                        rp.record_id
                    );
                    let ack = ReplicateAck {
                        record_id: rp.record_id.clone(),
                        accepted: true,
                        reason: String::new(),
                    };
                    let ack_frame =
                        Frame::new(MsgType::ReplicateAck, serde_json::to_vec(&ack)?);
                    frame::write_frame(send, &ack_frame).await?;
                }
                MsgType::ReplicateAck => {
                    let ra: ReplicateAck = f.decode_payload()?;
                    debug!(
                        "ReplicateAck from {}: record_id={} accepted={}",
                        &peer_node_id[..8.min(peer_node_id.len())],
                        ra.record_id,
                        ra.accepted
                    );
                }
                MsgType::PeerExchange => {
                    let pe: PeerExchange = f.decode_payload()?;
                    debug!(
                        "PeerExchange from {}: {} peers",
                        &peer_node_id[..8.min(peer_node_id.len())],
                        pe.peers.len()
                    );
                    // Respond with our known peers
                    let our_peers: Vec<NodeInfo> = routing_table
                        .read()
                        .await
                        .list()
                        .iter()
                        .filter(|e| e.node_id != peer_node_id)
                        .map(|e| NodeInfo {
                            id: e.node_id.clone(),
                            addr: e.addr.clone(),
                        })
                        .collect();
                    let reply = PeerExchange { peers: our_peers };
                    let reply_frame =
                        Frame::new(MsgType::PeerExchange, serde_json::to_vec(&reply)?);
                    frame::write_frame(send, &reply_frame).await?;
                }
                MsgType::Goodbye => {
                    let gb: Goodbye = f.decode_payload()?;
                    info!(
                        "Peer {} sent Goodbye: {}",
                        &peer_node_id[..8.min(peer_node_id.len())],
                        gb.reason
                    );
                    return Ok(());
                }
                _ => {
                    debug!("Ignoring unknown message type: {:?}", f.msg_type);
                }
            },
            None => {
                info!(
                    "Stream from {} closed",
                    &peer_node_id[..8.min(peer_node_id.len())]
                );
                return Ok(());
            }
        }
    }
}

/// Helper: write current routing table to peers.json.
/// Acquires peers_file_mutex to prevent concurrent writes.
async fn write_peers_file(routing_table: &Arc<RwLock<RoutingTable>>, data_dir: &Path, peers_file_mutex: &Arc<Mutex<()>>) {
    let _lock = peers_file_mutex.lock().await;
    let rt = routing_table.read().await;
    let peers: Vec<serde_json::Value> = rt
        .list()
        .iter()
        .map(|e| {
            serde_json::json!({
                "node_id": e.node_id,
                "addr": e.addr,
                "roles": e.roles,
                "last_seen": e.last_seen,
            })
        })
        .collect();
    let peers_json = serde_json::to_string_pretty(&peers).unwrap_or_default();
    let _ = std::fs::write(data_dir.join("peers.json"), peers_json);
}

fn abs_diff(a: u8, b: u8) -> u8 {
    a.abs_diff(b)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
