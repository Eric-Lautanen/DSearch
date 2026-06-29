use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A single cached search result entry.
#[derive(Debug, Clone)]
struct CacheEntry {
    results_json: String,
    inserted_at: Instant,
}

/// Thread-safe search result cache.
///
/// Caches the JSON response body of search queries to avoid
/// redundant store scans for repeated queries. Each entry has
/// a configurable TTL; expired entries are lazily evicted on
/// lookup.
pub struct SearchCache {
    entries: Mutex<HashMap<String, CacheEntry>>,
    ttl: Duration,
    max_entries: usize,
}

impl SearchCache {
    /// Create a new search cache with the given TTL and maximum entry count.
    pub fn new(ttl: Duration, max_entries: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            ttl,
            max_entries,
        }
    }

    /// Look up a cached result by query key.
    /// Returns the cached JSON string if found and not expired.
    /// Expired entries are removed on access (lazy eviction).
    pub fn get(&self, key: &str) -> Option<String> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = entries.get(key) {
            if entry.inserted_at.elapsed() < self.ttl {
                return Some(entry.results_json.clone());
            }
            // Expired — remove it
            entries.remove(key);
        }
        None
    }

    /// Insert a search result into the cache.
    /// If the cache is at capacity, evicts the oldest entry first.
    pub fn insert(&self, key: &str, results_json: &str) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());

        // Evict expired entries first
        let ttl = self.ttl;
        entries.retain(|_, v| v.inserted_at.elapsed() < ttl);

        // If still at capacity, evict the oldest entry
        while entries.len() >= self.max_entries {
            let oldest_key = entries
                .iter()
                .min_by_key(|(_, v)| v.inserted_at)
                .map(|(k, _)| k.clone());
            if let Some(k) = oldest_key {
                entries.remove(&k);
            } else {
                break;
            }
        }

        entries.insert(
            key.to_string(),
            CacheEntry {
                results_json: results_json.to_string(),
                inserted_at: Instant::now(),
            },
        );
    }

    /// Remove a specific entry from the cache.
    pub fn invalidate(&self, key: &str) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.remove(key);
    }

    /// Clear all entries from the cache.
    pub fn clear(&self) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.clear();
    }

    /// Return the number of entries currently in the cache (including possibly expired).
    pub fn len(&self) -> usize {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.len()
    }

    /// Return whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_insert_and_get() {
        let cache = SearchCache::new(Duration::from_secs(60), 100);
        cache.insert("rust", r#"{"results":[]}"#);
        assert_eq!(cache.get("rust"), Some(r#"{"results":[]}"#.to_string()));
        assert_eq!(cache.get("python"), None);
    }

    #[test]
    fn cache_expiry() {
        let cache = SearchCache::new(Duration::from_millis(10), 100);
        cache.insert("rust", r#"{"results":[]}"#);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(cache.get("rust"), None);
    }

    #[test]
    fn cache_max_entries_evicts_oldest() {
        let cache = SearchCache::new(Duration::from_secs(60), 2);
        cache.insert("a", "1");
        // Small sleep to ensure different insertion times
        std::thread::sleep(Duration::from_millis(1));
        cache.insert("b", "2");
        std::thread::sleep(Duration::from_millis(1));
        cache.insert("c", "3"); // should evict "a"
        assert_eq!(cache.get("a"), None);
        assert_eq!(cache.get("b"), Some("2".to_string()));
        assert_eq!(cache.get("c"), Some("3".to_string()));
    }

    #[test]
    fn cache_invalidate() {
        let cache = SearchCache::new(Duration::from_secs(60), 100);
        cache.insert("rust", r#"{"results":[]}"#);
        cache.invalidate("rust");
        assert_eq!(cache.get("rust"), None);
    }

    #[test]
    fn cache_clear() {
        let cache = SearchCache::new(Duration::from_secs(60), 100);
        cache.insert("a", "1");
        cache.insert("b", "2");
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn cache_overwrite_existing_key() {
        let cache = SearchCache::new(Duration::from_secs(60), 100);
        cache.insert("rust", "old");
        cache.insert("rust", "new");
        assert_eq!(cache.get("rust"), Some("new".to_string()));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn cache_len_tracking() {
        let cache = SearchCache::new(Duration::from_secs(60), 100);
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        cache.insert("a", "1");
        assert_eq!(cache.len(), 1);
        assert!(!cache.is_empty());
        cache.insert("b", "2");
        assert_eq!(cache.len(), 2);
        cache.invalidate("a");
        assert_eq!(cache.len(), 1);
        cache.clear();
        assert_eq!(cache.len(), 0);
        assert!(cache.is_empty());
    }

    #[test]
    fn cache_invalidate_nonexistent_key() {
        let cache = SearchCache::new(Duration::from_secs(60), 100);
        cache.insert("a", "1");
        cache.invalidate("nonexistent");
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get("a"), Some("1".to_string()));
    }

    #[test]
    fn cache_multiple_inserts_same_key() {
        let cache = SearchCache::new(Duration::from_secs(60), 100);
        for i in 0..5 {
            cache.insert("key", &format!("value{}", i));
        }
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get("key"), Some("value4".to_string()));
    }
}
