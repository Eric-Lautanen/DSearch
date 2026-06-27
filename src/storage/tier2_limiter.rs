/// Tier 2 write-rate limiting — caps announcement churn per peer.
/// High-frequency interval scrape jobs across many nodes stress write
/// throughput before they stress storage size, so we limit per-peer
/// announcement writes.
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Default max Tier 2 writes per peer per minute.
pub const DEFAULT_TIER2_WRITES_PER_MIN: u32 = 60;

struct PeerWriteCounter {
    count: u32,
    window_start: Instant,
}

/// Per-peer Tier 2 write rate limiter.
pub struct Tier2RateLimiter {
    peers: HashMap<String, PeerWriteCounter>,
    max_per_minute: u32,
    window: Duration,
}

impl Tier2RateLimiter {
    pub fn new(max_per_minute: u32) -> Self {
        Self {
            peers: HashMap::new(),
            max_per_minute,
            window: Duration::from_secs(60),
        }
    }

    /// Check if a write from the given peer is allowed.
    /// Returns true if allowed, false if rate-limited.
    pub fn allow_write(&mut self, peer_id: &str) -> bool {
        let now = Instant::now();
        let counter = self.peers.entry(peer_id.to_string())
            .or_insert(PeerWriteCounter {
                count: 0,
                window_start: now,
            });

        // Reset window if expired
        if now.duration_since(counter.window_start) >= self.window {
            counter.count = 0;
            counter.window_start = now;
        }

        if counter.count < self.max_per_minute {
            counter.count += 1;
            true
        } else {
            false
        }
    }

    /// Prune counters for peers whose windows have expired.
    pub fn prune(&mut self) {
        let now = Instant::now();
        self.peers.retain(|_, counter| {
            now.duration_since(counter.window_start) < self.window
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_writes_within_limit() {
        let mut limiter = Tier2RateLimiter::new(5);
        for _ in 0..5 {
            assert!(limiter.allow_write("peer1"));
        }
        assert!(!limiter.allow_write("peer1"));
    }

    #[test]
    fn different_peers_independent() {
        let mut limiter = Tier2RateLimiter::new(2);
        assert!(limiter.allow_write("peer1"));
        assert!(limiter.allow_write("peer1"));
        assert!(!limiter.allow_write("peer1"));
        assert!(limiter.allow_write("peer2"));
        assert!(limiter.allow_write("peer2"));
        assert!(!limiter.allow_write("peer2"));
    }

    #[test]
    fn prune_removes_expired() {
        let mut limiter = Tier2RateLimiter::new(100);
        limiter.allow_write("peer1");
        // Manually expire the window
        if let Some(counter) = limiter.peers.get_mut("peer1") {
            counter.window_start = Instant::now() - Duration::from_secs(120);
        }
        limiter.prune();
        assert!(limiter.peers.is_empty());
    }
}
