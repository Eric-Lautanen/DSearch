use serde::{Deserialize, Serialize};

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgType {
    Handshake = 0x01,
    HandshakeAck = 0x02,
    Ping = 0x03,
    Pong = 0x04,
    FindNode = 0x05,
    FindNodeReply = 0x06,
    Announce = 0x07,
    AnnounceAck = 0x08,
    SearchQuery = 0x09,
    SearchReply = 0x0A,
    RecordFetch = 0x0B,
    RecordReply = 0x0C,
    ReplicatePush = 0x0D,
    ReplicateAck = 0x0E,
    PeerExchange = 0x0F,
    Goodbye = 0xFF,
}

impl MsgType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Self::Handshake),
            0x02 => Some(Self::HandshakeAck),
            0x03 => Some(Self::Ping),
            0x04 => Some(Self::Pong),
            0x05 => Some(Self::FindNode),
            0x06 => Some(Self::FindNodeReply),
            0x07 => Some(Self::Announce),
            0x08 => Some(Self::AnnounceAck),
            0x09 => Some(Self::SearchQuery),
            0x0A => Some(Self::SearchReply),
            0x0B => Some(Self::RecordFetch),
            0x0C => Some(Self::RecordReply),
            0x0D => Some(Self::ReplicatePush),
            0x0E => Some(Self::ReplicateAck),
            0x0F => Some(Self::PeerExchange),
            0xFF => Some(Self::Goodbye),
            _ => None,
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

pub const PROTOCOL_VERSION: u8 = 1;
pub const MAX_PAYLOAD_SIZE: u32 = 1_048_576; // 1 MB

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Handshake {
    pub version: u8,
    pub node_id: String,
    pub roles: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeAck {
    pub version: u8,
    pub node_id: String,
    pub roles: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ping {
    pub nonce: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pong {
    pub nonce: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindNode {
    pub target_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindNodeReply {
    pub nodes: Vec<NodeInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub id: String,
    pub addr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goodbye {
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Announce {
    pub record_id: String,
    pub source_hash: String,
    pub schema: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub holder_addr: String,
    pub expires_at: u64,
    #[serde(default)]
    pub sig: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnounceAck {
    pub record_id: String,
    pub accepted: bool,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub query: String,
    #[serde(default)]
    pub schema: Option<String>,
    #[serde(default = "default_search_limit")]
    pub limit: u32,
    pub requester_id: String,
    #[serde(default)]
    pub reply_to: String,
}

fn default_search_limit() -> u32 {
    20
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchReply {
    pub query: String,
    pub results: Vec<SearchResultEntry>,
    pub from_node: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultEntry {
    pub record_id: String,
    pub schema: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub holder_addr: String,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordFetch {
    pub record_id: String,
    pub requester_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordReply {
    pub record_id: String,
    #[serde(default)]
    pub record_json: Option<String>,
    #[serde(default)]
    pub not_found: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicatePush {
    pub record_id: String,
    pub record_json: String,
    pub source_hash: String,
    #[serde(default)]
    pub sig: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicateAck {
    pub record_id: String,
    pub accepted: bool,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerExchange {
    pub peers: Vec<NodeInfo>,
}
