use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const K_BUCKET_SIZE: usize = 20;
const MAX_NODE_ID_BITS: usize = 256;

/// A node entry in the routing table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingEntry {
    pub node_id: String,
    pub addr: String,
    pub roles: Vec<String>,
    pub last_seen: u64,
}

/// Kademlia-style routing table (Tier 1).
/// Uses XOR distance for bucket placement.
pub struct RoutingTable {
    /// node_id of the local node (hex string)
    local_id: String,
    /// Flat list of known peers (simplified for Phase 1 — full k-buckets later)
    buckets: BTreeMap<String, RoutingEntry>,
}

impl RoutingTable {
    pub fn new(local_id: String) -> Self {
        Self {
            local_id,
            buckets: BTreeMap::new(),
        }
    }

    /// XOR distance between two hex-encoded node IDs.
    pub fn xor_distance(a: &str, b: &str) -> [u8; 32] {
        let a_bytes = hex_to_bytes(a);
        let b_bytes = hex_to_bytes(b);
        let mut result = [0u8; 32];
        for i in 0..32 {
            if i < a_bytes.len() && i < b_bytes.len() {
                result[i] = a_bytes[i] ^ b_bytes[i];
            }
        }
        result
    }

    /// Insert or update a peer in the routing table.
    pub fn insert(&mut self, entry: RoutingEntry) {
        self.buckets.insert(entry.node_id.clone(), entry);
    }

    /// Remove a peer from the routing table.
    pub fn remove(&mut self, node_id: &str) -> bool {
        self.buckets.remove(node_id).is_some()
    }

    /// Find the K closest peers to a target node_id.
    pub fn find_closest(&self, target_id: &str, k: usize) -> Vec<RoutingEntry> {
        let mut entries: Vec<&RoutingEntry> = self.buckets.values().collect();
        entries.sort_by_key(|e| {
            let dist = Self::xor_distance(&e.node_id, target_id);
            // Convert first 8 bytes of distance to u64 for comparison
            let mut arr = [0u8; 8];
            arr.copy_from_slice(&dist[..8]);
            u64::from_be_bytes(arr)
        });
        entries.into_iter().take(k).cloned().collect()
    }

    /// Get a peer by node_id.
    pub fn get(&self, node_id: &str) -> Option<&RoutingEntry> {
        self.buckets.get(node_id)
    }

    /// List all known peers.
    pub fn list(&self) -> Vec<&RoutingEntry> {
        self.buckets.values().collect()
    }

    /// Number of known peers.
    pub fn len(&self) -> usize {
        self.buckets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }

    pub fn local_id(&self) -> &str {
        &self.local_id
    }
}

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .filter_map(|i| hex.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_find() {
        let mut rt = RoutingTable::new("aa".repeat(32));
        rt.insert(RoutingEntry {
            node_id: "bb".repeat(32),
            addr: "127.0.0.1:7744".to_string(),
            roles: vec!["full".to_string()],
            last_seen: 0,
        });
        assert_eq!(rt.len(), 1);
        assert!(rt.get(&"bb".repeat(32)).is_some());
    }

    #[test]
    fn find_closest_returns_sorted() {
        let mut rt = RoutingTable::new("00".repeat(32));
        rt.insert(RoutingEntry {
            node_id: "01".repeat(32),
            addr: "1".to_string(),
            roles: vec![],
            last_seen: 0,
        });
        rt.insert(RoutingEntry {
            node_id: "ff".repeat(32),
            addr: "2".to_string(),
            roles: vec![],
            last_seen: 0,
        });
        let closest = rt.find_closest(&"00".repeat(32), 1);
        assert_eq!(closest.len(), 1);
        assert_eq!(closest[0].node_id, "01".repeat(32));
    }
}
