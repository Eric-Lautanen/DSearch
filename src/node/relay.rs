use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Tracks bandwidth usage per peer for relay traffic.
///
/// Relays forward traffic between nodes that cannot directly connect.
/// This module ensures no single peer can consume unlimited relay bandwidth,
/// enforcing per-peer quotas based on the configured limit.
pub struct RelayBandwidthAccount {
    accounts: Mutex<HashMap<String, BandwidthAccount>>,
    limit_bytes_per_window: u64,
    window: Duration,
}

struct BandwidthAccount {
    bytes_used: u64,
    reset_at: Instant,
}

impl RelayBandwidthAccount {
    /// Create a new bandwidth account tracker.
    ///
    /// - `limit_mbps`: Maximum megabits per second allowed per peer.
    /// - `window`: Rolling time window for the bandwidth limit.
    pub fn new(limit_mbps: u32, window: Duration) -> Self {
        // Convert Mbps to bytes per window
        // Mbps * 1_000_000 bits/s / 8 bits/byte * window_secs = bytes
        let window_secs = window.as_secs_f64();
        let limit_bytes_per_window = (limit_mbps as f64 * 1_000_000.0 / 8.0 * window_secs) as u64;

        Self {
            accounts: Mutex::new(HashMap::new()),
            limit_bytes_per_window,
            window,
        }
    }

    /// Check if a peer can relay `n` bytes.
    /// Returns `true` if the relay is within bandwidth limits.
    /// If allowed, the byte count is recorded against the peer's quota.
    pub fn allow(&self, peer_id: &str, bytes: u64) -> bool {
        let mut accounts = self.accounts.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();

        let account = accounts
            .entry(peer_id.to_string())
            .or_insert_with(|| BandwidthAccount {
                bytes_used: 0,
                reset_at: now + self.window,
            });

        // Reset the window if elapsed
        if now >= account.reset_at {
            account.bytes_used = 0;
            account.reset_at = now + self.window;
        }

        if account.bytes_used + bytes <= self.limit_bytes_per_window {
            account.bytes_used += bytes;
            true
        } else {
            false
        }
    }

    /// Record bytes relayed for a peer without checking the limit.
    /// Useful for tracking usage when the limit is enforced elsewhere.
    pub fn record(&self, peer_id: &str, bytes: u64) {
        let mut accounts = self.accounts.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();

        let account = accounts
            .entry(peer_id.to_string())
            .or_insert_with(|| BandwidthAccount {
                bytes_used: 0,
                reset_at: now + self.window,
            });

        if now >= account.reset_at {
            account.bytes_used = 0;
            account.reset_at = now + self.window;
        }

        account.bytes_used += bytes;
    }

    /// Return the remaining bytes a peer can relay in the current window.
    pub fn remaining(&self, peer_id: &str) -> u64 {
        let accounts = self.accounts.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();

        if let Some(account) = accounts.get(peer_id) {
            if now >= account.reset_at {
                self.limit_bytes_per_window
            } else {
                self.limit_bytes_per_window.saturating_sub(account.bytes_used)
            }
        } else {
            self.limit_bytes_per_window
        }
    }

    /// Prune expired accounts to prevent unbounded memory growth.
    pub fn prune_expired(&self) {
        let mut accounts = self.accounts.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        accounts.retain(|_, a| now < a.reset_at);
    }

    /// Return the number of tracked peers.
    pub fn len(&self) -> usize {
        let accounts = self.accounts.lock().unwrap_or_else(|e| e.into_inner());
        accounts.len()
    }

    /// Return whether no peers are tracked.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_within_limit() {
        let account = RelayBandwidthAccount::new(100, Duration::from_secs(1));
        // 100 Mbps = 12.5 MB/s = 12_500_000 bytes per 1s window
        assert!(account.allow("peer1", 1_000_000));
        assert!(account.allow("peer1", 1_000_000));
    }

    #[test]
    fn reject_over_limit() {
        let account = RelayBandwidthAccount::new(1, Duration::from_secs(1));
        // 1 Mbps = 125_000 bytes per 1s window
        assert!(account.allow("peer1", 100_000));
        assert!(account.allow("peer1", 25_000));
        assert!(!account.allow("peer1", 1)); // over limit
    }

    #[test]
    fn different_peers_independent() {
        let account = RelayBandwidthAccount::new(1, Duration::from_secs(1));
        assert!(account.allow("peer1", 100_000));
        assert!(account.allow("peer2", 100_000)); // separate quota
    }

    #[test]
    fn window_resets() {
        let account = RelayBandwidthAccount::new(1, Duration::from_millis(10));
        // 1 Mbps * 10ms = 1,250 bytes
        assert!(account.allow("peer1", 1_200));
        assert!(!account.allow("peer1", 100));
        std::thread::sleep(Duration::from_millis(15));
        assert!(account.allow("peer1", 1_000)); // window reset
    }

    #[test]
    fn remaining_bytes() {
        let account = RelayBandwidthAccount::new(1, Duration::from_secs(1));
        // 1 Mbps = 125_000 bytes per 1s window
        assert_eq!(account.remaining("peer1"), 125_000);
        account.allow("peer1", 25_000);
        assert_eq!(account.remaining("peer1"), 100_000);
    }

    #[test]
    fn prune_expired() {
        let account = RelayBandwidthAccount::new(1, Duration::from_millis(10));
        account.allow("peer1", 100);
        std::thread::sleep(Duration::from_millis(15));
        account.prune_expired();
        assert!(account.is_empty());
    }
}
