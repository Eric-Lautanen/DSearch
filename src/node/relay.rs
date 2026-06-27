/// Relay bandwidth accounting — persisted across restarts.
///
/// Tracks bytes relayed per time window, persisted to a JSON file
/// in the data directory so accounting survives process restarts.
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Window size for bandwidth accounting (1 hour).
const WINDOW_SECS: u64 = 3600;

/// A single bandwidth sample.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BandwidthSample {
    /// Unix timestamp of the sample.
    timestamp_secs: u64,
    /// Bytes relayed in this sample.
    bytes: u64,
}

/// Relay bandwidth accounting state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayBandwidthAccount {
    /// Rolling window of bandwidth samples.
    samples: VecDeque<BandwidthSample>,
    /// Maximum bandwidth in Mbps (from config).
    limit_mbps: u32,
}

impl RelayBandwidthAccount {
    pub fn new(limit_mbps: u32) -> Self {
        Self {
            samples: VecDeque::new(),
            limit_mbps,
        }
    }

    /// Record bytes relayed.
    pub fn record_bytes(&mut self, bytes: u64) {
        let now = now_secs();
        // Prune old samples first
        self.prune_old(now);
        // Add to the latest sample or create a new one
        if let Some(last) = self.samples.back_mut() {
            if now - last.timestamp_secs < 60 {
                // Within the same minute bucket
                last.bytes += bytes;
                return;
            }
        }
        self.samples.push_back(BandwidthSample {
            timestamp_secs: now,
            bytes,
        });
    }

    /// Get total bytes relayed in the current window.
    pub fn current_window_bytes(&self) -> u64 {
        let now = now_secs();
        self.samples.iter()
            .filter(|s| now.saturating_sub(s.timestamp_secs) < WINDOW_SECS)
            .map(|s| s.bytes)
            .sum()
    }

    /// Check if relaying is allowed within the bandwidth limit.
    pub fn allow_relay(&self) -> bool {
        if self.limit_mbps == 0 {
            return true; // unlimited
        }
        let current_bytes = self.current_window_bytes();
        let limit_bytes = (self.limit_mbps as u64) * 1_000_000 / 8 * WINDOW_SECS;
        current_bytes < limit_bytes
    }

    /// Prune samples older than the window.
    fn prune_old(&mut self, now: u64) {
        while let Some(front) = self.samples.front() {
            if now.saturating_sub(front.timestamp_secs) >= WINDOW_SECS {
                self.samples.pop_front();
            } else {
                break;
            }
        }
    }

    /// Persist accounting state to disk.
    pub fn save(&self, data_dir: &Path) -> Result<(), String> {
        let path = data_dir.join("relay_bandwidth.json");
        let json = serde_json::to_string(self)
            .map_err(|e| format!("Failed to serialize relay bandwidth: {}", e))?;
        std::fs::write(&path, json)
            .map_err(|e| format!("Failed to write relay_bandwidth.json: {}", e))
    }

    /// Load accounting state from disk.
    pub fn load(data_dir: &Path, limit_mbps: u32) -> Result<Self, String> {
        let path = data_dir.join("relay_bandwidth.json");
        if !path.exists() {
            return Ok(Self::new(limit_mbps));
        }
        let json = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read relay_bandwidth.json: {}", e))?;
        let mut account: Self = serde_json::from_str(&json)
            .map_err(|e| format!("Failed to parse relay_bandwidth.json: {}", e))?;
        account.limit_mbps = limit_mbps;
        account.prune_old(now_secs());
        Ok(account)
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
    fn new_account_is_empty() {
        let account = RelayBandwidthAccount::new(100);
        assert_eq!(account.current_window_bytes(), 0);
    }

    #[test]
    fn record_bytes_adds_up() {
        let mut account = RelayBandwidthAccount::new(100);
        account.record_bytes(1000);
        account.record_bytes(2000);
        assert_eq!(account.current_window_bytes(), 3000);
    }

    #[test]
    fn allow_relay_unlimited() {
        let account = RelayBandwidthAccount::new(0);
        assert!(account.allow_relay());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join("dsearch_test_relay_bw");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut account = RelayBandwidthAccount::new(100);
        account.record_bytes(5000);
        account.save(&dir).unwrap();

        let loaded = RelayBandwidthAccount::load(&dir, 100).unwrap();
        assert_eq!(loaded.current_window_bytes(), 5000);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
