use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Per-IP rate limiter for Tier 2 announcement writes.
///
/// Limits how many announcements a single source IP can submit
/// within a rolling time window. This prevents a single node
/// from flooding the Tier 2 index with spam announcements.
pub struct Tier2Limiter {
    buckets: Mutex<HashMap<String, TokenBucket>>,
    max_requests: u32,
    window: Duration,
}

struct TokenBucket {
    count: u32,
    reset_at: Instant,
}

impl Tier2Limiter {
    /// Create a new Tier2 rate limiter.
    ///
    /// - `max_requests`: maximum announcements allowed per IP within the window.
    /// - `window`: rolling time window for the rate limit.
    pub fn new(max_requests: u32, window: Duration) -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
            max_requests,
            window,
        }
    }

    /// Check whether a request from the given IP is allowed.
    /// Returns `true` if the request is within rate limits, `false` if rejected.
    /// A successful check increments the counter for this IP.
    pub fn allow(&self, ip: &str) -> bool {
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();

        let bucket = buckets
            .entry(ip.to_string())
            .or_insert_with(|| TokenBucket {
                count: 0,
                reset_at: now + self.window,
            });

        // Reset the bucket if the window has elapsed
        if now >= bucket.reset_at {
            bucket.count = 0;
            bucket.reset_at = now + self.window;
        }

        if bucket.count < self.max_requests {
            bucket.count += 1;
            true
        } else {
            false
        }
    }

    /// Return the remaining request count for a given IP.
    pub fn remaining(&self, ip: &str) -> u32 {
        let buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();

        if let Some(bucket) = buckets.get(ip) {
            if now >= bucket.reset_at {
                self.max_requests
            } else {
                self.max_requests.saturating_sub(bucket.count)
            }
        } else {
            self.max_requests
        }
    }

    /// Prune expired buckets to prevent unbounded memory growth.
    pub fn prune_expired(&self) {
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        buckets.retain(|_, b| now < b.reset_at);
    }

    /// Return the number of tracked IPs.
    pub fn len(&self) -> usize {
        let buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        buckets.len()
    }

    /// Return whether no IPs are tracked.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_within_limit() {
        let limiter = Tier2Limiter::new(3, Duration::from_secs(60));
        assert!(limiter.allow("1.2.3.4"));
        assert!(limiter.allow("1.2.3.4"));
        assert!(limiter.allow("1.2.3.4"));
        assert!(!limiter.allow("1.2.3.4")); // 4th request rejected
    }

    #[test]
    fn different_ips_independent() {
        let limiter = Tier2Limiter::new(1, Duration::from_secs(60));
        assert!(limiter.allow("1.1.1.1"));
        assert!(limiter.allow("2.2.2.2")); // different IP, separate bucket
    }

    #[test]
    fn window_resets() {
        let limiter = Tier2Limiter::new(1, Duration::from_millis(10));
        assert!(limiter.allow("1.1.1.1"));
        assert!(!limiter.allow("1.1.1.1"));
        std::thread::sleep(Duration::from_millis(15));
        assert!(limiter.allow("1.1.1.1")); // window reset
    }

    #[test]
    fn remaining_count() {
        let limiter = Tier2Limiter::new(5, Duration::from_secs(60));
        assert_eq!(limiter.remaining("1.1.1.1"), 5);
        limiter.allow("1.1.1.1");
        assert_eq!(limiter.remaining("1.1.1.1"), 4);
    }

    #[test]
    fn prune_expired() {
        let limiter = Tier2Limiter::new(1, Duration::from_millis(10));
        limiter.allow("1.1.1.1");
        std::thread::sleep(Duration::from_millis(15));
        limiter.prune_expired();
        assert!(limiter.is_empty());
    }

    #[test]
    fn len_tracking() {
        let limiter = Tier2Limiter::new(10, Duration::from_secs(60));
        assert!(limiter.is_empty());
        assert_eq!(limiter.len(), 0);
        limiter.allow("1.1.1.1");
        assert_eq!(limiter.len(), 1);
        assert!(!limiter.is_empty());
        limiter.allow("2.2.2.2");
        assert_eq!(limiter.len(), 2);
    }

    #[test]
    fn remaining_unknown_ip() {
        let limiter = Tier2Limiter::new(5, Duration::from_secs(60));
        // Unknown IP should return full allowance
        assert_eq!(limiter.remaining("unknown"), 5);
    }

    #[test]
    fn remaining_after_window_reset() {
        let limiter = Tier2Limiter::new(5, Duration::from_millis(10));
        limiter.allow("1.1.1.1");
        limiter.allow("1.1.1.1");
        assert_eq!(limiter.remaining("1.1.1.1"), 3);
        std::thread::sleep(Duration::from_millis(15));
        // After window expires, remaining should be back to max
        assert_eq!(limiter.remaining("1.1.1.1"), 5);
    }

    #[test]
    fn len_after_prune() {
        let limiter = Tier2Limiter::new(10, Duration::from_millis(10));
        limiter.allow("1.1.1.1");
        limiter.allow("2.2.2.2");
        assert_eq!(limiter.len(), 2);
        std::thread::sleep(Duration::from_millis(15));
        limiter.prune_expired();
        assert_eq!(limiter.len(), 0);
        assert!(limiter.is_empty());
    }
}
