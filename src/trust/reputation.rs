/// Peer reputation system — Phase 9.
///
/// Tracks per-peer penalties for misbehavior (flood, malformed, slow,
/// signature/hash failures). Penalties decay over 24h. Bans are manual-only.
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};

/// Penalty points assigned for each infraction type.
pub const PENALTY_FLOOD: u32 = 50;
pub const PENALTY_MALFORMED: u32 = 30;
pub const PENALTY_SLOW: u32 = 10;
pub const PENALTY_BAD_SIGNATURE: u32 = 40;
pub const PENALTY_BAD_HASH: u32 = 40;

/// Total penalty score at which a peer is effectively banned.
/// Bans are manual-only, but scores above this threshold trigger warnings
/// and reduced priority in peer selection.
pub const REPUTATION_THRESHOLD: u32 = 100;

/// Seconds over which penalty points fully decay.
const DECAY_PERIOD_SECS: u64 = 86400; // 24 hours

/// A single penalty event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PenaltyEvent {
    pub reason: PenaltyReason,
    pub points: u32,
    pub timestamp_secs: u64,
}

/// Reasons a peer can be penalized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PenaltyReason {
    Flood,
    Malformed,
    Slow,
    BadSignature,
    BadHash,
    BadRecordId,
}

impl std::fmt::Display for PenaltyReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PenaltyReason::Flood => write!(f, "flood"),
            PenaltyReason::Malformed => write!(f, "malformed"),
            PenaltyReason::Slow => write!(f, "slow"),
            PenaltyReason::BadSignature => write!(f, "bad_signature"),
            PenaltyReason::BadHash => write!(f, "bad_hash"),
            PenaltyReason::BadRecordId => write!(f, "bad_record_id"),
        }
    }
}

/// Per-peer reputation state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerReputation {
    pub node_id: String,
    pub penalties: Vec<PenaltyEvent>,
    pub banned: bool,
    pub ban_reason: Option<String>,
}

impl PeerReputation {
    pub fn new(node_id: String) -> Self {
        Self {
            node_id,
            penalties: Vec::new(),
            banned: false,
            ban_reason: None,
        }
    }

    /// Add a penalty event.
    pub fn add_penalty(&mut self, reason: PenaltyReason, points: u32) {
        let now = now_secs();
        self.penalties.push(PenaltyEvent {
            reason,
            points,
            timestamp_secs: now,
        });
    }

    /// Compute the current score with decay.
    /// Penalty points decay linearly to zero over DECAY_PERIOD_SECS.
    pub fn current_score(&self) -> u32 {
        let now = now_secs();
        self.penalties
            .iter()
            .map(|p| {
                let age = now.saturating_sub(p.timestamp_secs);
                if age >= DECAY_PERIOD_SECS {
                    0
                } else {
                    let decay_fraction = (DECAY_PERIOD_SECS - age) as f64 / DECAY_PERIOD_SECS as f64;
                    (p.points as f64 * decay_fraction).round() as u32
                }
            })
            .sum()
    }

    /// Whether this peer's score exceeds the reputation threshold.
    pub fn is_distrusted(&self) -> bool {
        self.banned || self.current_score() >= REPUTATION_THRESHOLD
    }

    /// Manually ban a peer.
    pub fn ban(&mut self, reason: String) {
        self.banned = true;
        self.ban_reason = Some(reason);
    }

    /// Manually unban a peer.
    pub fn unban(&mut self) {
        self.banned = false;
        self.ban_reason = None;
    }

    /// Prune fully-decayed penalty events to keep memory bounded.
    pub fn prune_expired_penalties(&mut self) {
        let now = now_secs();
        self.penalties.retain(|p| now.saturating_sub(p.timestamp_secs) < DECAY_PERIOD_SECS);
    }
}

/// The global peer reputation table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationTable {
    peers: HashMap<String, PeerReputation>,
}

impl ReputationTable {
    pub fn new() -> Self {
        Self {
            peers: HashMap::new(),
        }
    }

    /// Record a penalty for a peer.
    pub fn penalize(&mut self, node_id: &str, reason: PenaltyReason, points: u32) {
        let entry = self.peers.entry(node_id.to_string())
            .or_insert_with(|| PeerReputation::new(node_id.to_string()));
        entry.add_penalty(reason, points);
    }

    /// Get the reputation for a peer (creates empty if missing).
    pub fn get(&mut self, node_id: &str) -> &PeerReputation {
        self.peers.entry(node_id.to_string())
            .or_insert_with(|| PeerReputation::new(node_id.to_string()))
    }

    /// Check if a peer is banned or distrusted.
    pub fn is_distrusted(&self, node_id: &str) -> bool {
        self.peers.get(node_id).map_or(false, |p| p.is_distrusted())
    }

    /// Manually ban a peer.
    pub fn ban(&mut self, node_id: &str, reason: String) {
        let entry = self.peers.entry(node_id.to_string())
            .or_insert_with(|| PeerReputation::new(node_id.to_string()));
        entry.ban(reason);
    }

    /// Manually unban a peer.
    pub fn unban(&mut self, node_id: &str) {
        if let Some(entry) = self.peers.get_mut(node_id) {
            entry.unban();
        }
    }

    /// Prune fully-decayed penalties across all peers.
    /// Also removes peers with no remaining penalties and no ban.
    pub fn prune(&mut self) {
        for p in self.peers.values_mut() {
            p.prune_expired_penalties();
        }
        self.peers.retain(|_, p| !p.penalties.is_empty() || p.banned);
    }

    /// List all peers with their current scores.
    pub fn list(&self) -> Vec<(&str, u32, bool)> {
        self.peers.iter()
            .map(|(id, rep)| (id.as_str(), rep.current_score(), rep.banned))
            .collect()
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn penalty_adds_score() {
        let mut rep = PeerReputation::new("test-peer".to_string());
        rep.add_penalty(PenaltyReason::Flood, PENALTY_FLOOD);
        assert!(rep.current_score() > 0);
        assert_eq!(rep.current_score(), PENALTY_FLOOD);
    }

    #[test]
    fn score_decays_over_time() {
        let mut rep = PeerReputation::new("test-peer".to_string());
        // Manually insert a penalty 12 hours ago
        let now = now_secs();
        rep.penalties.push(PenaltyEvent {
            reason: PenaltyReason::Flood,
            points: PENALTY_FLOOD,
            timestamp_secs: now - 43200, // 12 hours ago
        });
        // Score should be roughly half
        let score = rep.current_score();
        assert!(score > 0);
        assert!(score < PENALTY_FLOOD);
        assert!(score >= PENALTY_FLOOD / 2 - 1);
    }

    #[test]
    fn fully_decayed_penalty_is_zero() {
        let mut rep = PeerReputation::new("test-peer".to_string());
        let now = now_secs();
        rep.penalties.push(PenaltyEvent {
            reason: PenaltyReason::Flood,
            points: PENALTY_FLOOD,
            timestamp_secs: now - DECAY_PERIOD_SECS - 1,
        });
        assert_eq!(rep.current_score(), 0);
    }

    #[test]
    fn ban_manual_only() {
        let mut rep = PeerReputation::new("test-peer".to_string());
        rep.add_penalty(PenaltyReason::Flood, PENALTY_FLOOD);
        rep.add_penalty(PenaltyReason::BadSignature, PENALTY_BAD_SIGNATURE);
        // 50 + 40 = 90, still under threshold
        assert!(!rep.is_distrusted());
        rep.add_penalty(PenaltyReason::Malformed, PENALTY_MALFORMED);
        // 90 + 30 = 120 >= 100, now distrusted
        assert!(rep.is_distrusted());
        assert!(!rep.banned); // but not manually banned
        rep.ban("flooding".to_string());
        assert!(rep.banned);
        rep.unban();
        assert!(!rep.banned);
    }

    #[test]
    fn prune_removes_expired() {
        let mut rep = PeerReputation::new("test-peer".to_string());
        let now = now_secs();
        rep.penalties.push(PenaltyEvent {
            reason: PenaltyReason::Flood,
            points: PENALTY_FLOOD,
            timestamp_secs: now - DECAY_PERIOD_SECS - 1,
        });
        rep.prune_expired_penalties();
        assert!(rep.penalties.is_empty());
    }

    #[test]
    fn reputation_table_penalize_and_check() {
        let mut table = ReputationTable::new();
        table.penalize("peer1", PenaltyReason::Malformed, PENALTY_MALFORMED);
        assert!(!table.is_distrusted("peer1")); // 30 < 100
        table.penalize("peer1", PenaltyReason::BadSignature, PENALTY_BAD_SIGNATURE);
        // 30 + 40 = 70, still under 100
        assert!(!table.is_distrusted("peer1"));
        table.penalize("peer1", PenaltyReason::Flood, PENALTY_FLOOD);
        // 70 + 50 = 120 >= 100
        assert!(table.is_distrusted("peer1"));
    }

    #[test]
    fn reputation_table_ban_unban() {
        let mut table = ReputationTable::new();
        table.ban("peer1", "abuse".to_string());
        assert!(table.is_distrusted("peer1"));
        table.unban("peer1");
        assert!(!table.is_distrusted("peer1"));
    }

    #[test]
    fn prune_removes_empty_peers() {
        let mut table = ReputationTable::new();
        let now = now_secs();
        // Add a peer with an expired penalty
        let mut rep = PeerReputation::new("peer1".to_string());
        rep.penalties.push(PenaltyEvent {
            reason: PenaltyReason::Flood,
            points: PENALTY_FLOOD,
            timestamp_secs: now - DECAY_PERIOD_SECS - 1,
        });
        table.peers.insert("peer1".to_string(), rep);
        table.prune();
        assert!(!table.peers.contains_key("peer1"));
    }
}
