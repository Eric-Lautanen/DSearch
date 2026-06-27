/// Search result cache — short TTL so repeat queries don't re-trigger
/// full K=20 fan-out on every keystroke.
use std::collections::HashMap;
use std::time::{Duration, Instant};
use crate::model::ContentRecord;

/// Default cache TTL in seconds.
pub const DEFAULT_CACHE_TTL_SECS: u64 = 30;

/// Maximum number of cached query results.
const MAX_CACHE_ENTRIES: usize = 100;

struct CacheEntry {
    results: Vec<ContentRecord>,
    inserted_at: Instant,
}

/// A simple TTL-based search result cache.
pub struct SearchCache {
    entries: HashMap<String, CacheEntry>,
    ttl: Duration,
}

impl SearchCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            entries: HashMap::new(),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Look up a cached result. Returns None if not cached or expired.
    pub fn get(&self, query_key: &str) -> Option<&Vec<ContentRecord>> {
        self.entries.get(query_key).and_then(|entry| {
            if entry.inserted_at.elapsed() < self.ttl {
                Some(&entry.results)
            } else {
                None
            }
        })
    }

    /// Insert a search result into the cache.
    /// Evicts the oldest entry if the cache is full.
    pub fn insert(&mut self, query_key: String, results: Vec<ContentRecord>) {
        // Evict expired entries first
        self.evict_expired();

        // If still full, evict the oldest entry
        if self.entries.len() >= MAX_CACHE_ENTRIES {
            if let Some(oldest_key) = self.entries.iter()
                .min_by_key(|(_, v)| v.inserted_at)
                .map(|(k, _)| k.clone())
            {
                self.entries.remove(&oldest_key);
            }
        }

        self.entries.insert(query_key, CacheEntry {
            results,
            inserted_at: Instant::now(),
        });
    }

    /// Remove all expired entries.
    fn evict_expired(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_, entry| now.duration_since(entry.inserted_at) < self.ttl);
    }

    /// Number of cached entries (including potentially expired).
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_record(id: &str) -> ContentRecord {
        ContentRecord {
            id: id.to_string(),
            source_url: format!("https://example.com/{}", id),
            source_hash: format!("hash_{}", id),
            schema: "generic/kv".to_string(),
            tags: vec![],
            body: format!("Body of {}", id),
            created_at: 1000,
            expires_at: 9999,
            scrape_source: crate::model::ScrapeSource::Url,
            refresh_policy: crate::model::RefreshPolicy::Once,
            sig: "".to_string(),
        }
    }

    #[test]
    fn cache_hit_within_ttl() {
        let mut cache = SearchCache::new(30);
        let records = vec![make_record("r1")];
        cache.insert("test query".to_string(), records.clone());
        let hit = cache.get("test query");
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().len(), 1);
    }

    #[test]
    fn cache_miss_unknown_key() {
        let cache = SearchCache::new(30);
        assert!(cache.get("nonexistent").is_none());
    }

    #[test]
    fn cache_evicts_on_capacity() {
        let mut cache = SearchCache::new(30);
        for i in 0..=MAX_CACHE_ENTRIES {
            cache.insert(format!("query_{}", i), vec![make_record(&format!("r{}", i))]);
        }
        // Should be at most MAX_CACHE_ENTRIES
        assert!(cache.len() <= MAX_CACHE_ENTRIES);
    }
}
