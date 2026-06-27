/// Jittered re-announcement — spreads pinned-record re-announce at TTL/2
/// with random jitter per record, avoiding an announcement storm hitting
/// the same K-closest peers simultaneously on rejoin.
use rand::Rng;

/// Compute a jittered re-announce delay for a record.
/// `ttl_secs` is the record's time-to-live.
/// `max_jitter_secs` is the maximum random jitter to add (default: 300 = 5 min).
/// Returns the delay in seconds from now until the re-announce should fire.
pub fn jittered_reannounce_delay(ttl_secs: u64, max_jitter_secs: u64) -> u64 {
    let base = ttl_secs / 2;
    let jitter = rand::thread_rng().gen_range(0..max_jitter_secs.max(1));
    base.saturating_add(jitter)
}

/// Compute jittered re-announce delay with default 5-minute jitter.
pub fn jittered_reannounce_delay_default(ttl_secs: u64) -> u64 {
    jittered_reannounce_delay(ttl_secs, 300)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_is_near_half_ttl() {
        // With jitter=0, delay should be exactly TTL/2
        let delay = jittered_reannounce_delay(3600, 0);
        assert_eq!(delay, 1800);
    }

    #[test]
    fn delay_includes_jitter() {
        // With jitter, delay should be in [TTL/2, TTL/2 + max_jitter)
        let delay = jittered_reannounce_delay(3600, 300);
        assert!(delay >= 1800);
        assert!(delay < 2100); // 1800 + 300
    }

    #[test]
    fn zero_ttl_gives_zero() {
        let delay = jittered_reannounce_delay(0, 300);
        // 0/2 = 0, plus jitter
        assert!(delay < 301);
    }
}
