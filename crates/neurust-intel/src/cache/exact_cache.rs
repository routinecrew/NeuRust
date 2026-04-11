//! # Exact Cache
//!
//! Lock-free concurrent hash map cache using `DashMap<u64, CacheEntry>`.
//! Supports TTL expiration, hit counting, and LFU eviction.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use crate::contracts::UnifiedResponse;

/// A single cache entry with metadata for eviction and warming.
pub struct CacheEntry {
    /// The cached response.
    pub response: UnifiedResponse,
    /// Number of times this entry has been hit.
    pub hit_count: AtomicU64,
    /// When this entry was created.
    pub created_at: Instant,
    /// Time-to-live for this entry.
    pub ttl: Duration,
}

impl CacheEntry {
    /// Check whether this entry has expired.
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() > self.ttl
    }
}

/// Serializable entry for cache dump/warm operations.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CacheDumpEntry {
    /// The cache key hash.
    pub hash: u64,
    /// The cached response.
    pub response: UnifiedResponse,
    /// Hit count at dump time.
    pub hit_count: u64,
    /// TTL in seconds (for restoration).
    pub ttl_secs: u64,
}

/// Concurrent exact-match cache backed by DashMap.
pub struct ExactCache {
    map: DashMap<u64, CacheEntry>,
    max_entries: usize,
    default_ttl: Duration,
}

impl ExactCache {
    /// Create a new exact cache with given capacity and default TTL.
    pub fn new(max_entries: usize, default_ttl: Duration) -> Self {
        Self {
            map: DashMap::with_capacity(max_entries),
            max_entries,
            default_ttl,
        }
    }

    /// Look up a cached response by hash.
    ///
    /// Returns `None` if the entry does not exist or has expired.
    /// Increments the hit count on successful lookup.
    pub fn get(&self, hash: u64) -> Option<UnifiedResponse> {
        let entry = self.map.get(&hash)?;

        if entry.is_expired() {
            drop(entry);
            self.map.remove(&hash);
            return None;
        }

        entry.hit_count.fetch_add(1, Ordering::Relaxed);
        Some(entry.response.clone())
    }

    /// Insert a response into the cache.
    ///
    /// Triggers LFU eviction if the cache exceeds `max_entries`.
    pub fn insert(&self, hash: u64, response: UnifiedResponse) {
        self.insert_with_ttl(hash, response, self.default_ttl);
    }

    /// Insert with a specific TTL.
    pub fn insert_with_ttl(&self, hash: u64, response: UnifiedResponse, ttl: Duration) {
        // Evict until under capacity (loop handles concurrent insert race)
        let mut evict_attempts = 0;
        while self.map.len() >= self.max_entries && evict_attempts < 5 {
            super::eviction::evict_lfu(&self.map);
            evict_attempts += 1;
        }

        self.map.insert(
            hash,
            CacheEntry {
                response,
                hit_count: AtomicU64::new(0),
                created_at: Instant::now(),
                ttl,
            },
        );
    }

    /// Insert a pre-warmed entry (from cache dump) with a given hit count.
    pub fn insert_warm(&self, hash: u64, response: UnifiedResponse, hit_count: u64) {
        let mut evict_attempts = 0;
        while self.map.len() >= self.max_entries && evict_attempts < 5 {
            super::eviction::evict_lfu(&self.map);
            evict_attempts += 1;
        }

        self.map.insert(
            hash,
            CacheEntry {
                response,
                hit_count: AtomicU64::new(hit_count),
                created_at: Instant::now(),
                ttl: self.default_ttl,
            },
        );
    }

    /// Return the top N entries by hit count, for cache warming/dump.
    pub fn top_by_hit_count(&self, n: usize) -> Vec<CacheDumpEntry> {
        let mut entries: Vec<CacheDumpEntry> = self
            .map
            .iter()
            .filter(|e| !e.value().is_expired())
            .map(|e| CacheDumpEntry {
                hash: *e.key(),
                response: e.value().response.clone(),
                hit_count: e.value().hit_count.load(Ordering::Relaxed),
                ttl_secs: e.value().ttl.as_secs(),
            })
            .collect();

        entries.sort_by(|a, b| b.hit_count.cmp(&a.hit_count));
        entries.truncate(n);
        entries
    }

    /// Current number of entries in the cache.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Remove all entries from the cache.
    pub fn clear(&self) {
        self.map.clear();
    }

    /// Get a reference to the internal DashMap (for eviction).
    #[allow(dead_code)]
    pub(crate) fn inner_map(&self) -> &DashMap<u64, CacheEntry> {
        &self.map
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::*;

    fn make_response(content: &str) -> UnifiedResponse {
        UnifiedResponse {
            content: content.to_string(),
            model: "gpt-4o".to_string(),
            usage: TokenUsage::default(),
            provider_id: "openai".to_string(),
            latency_ms: 100,
            upstream_id: None,
        }
    }

    #[test]
    fn test_insert_and_get() {
        let cache = ExactCache::new(100, Duration::from_secs(300));
        let resp = make_response("Hello world");

        cache.insert(42, resp.clone());
        let result = cache.get(42);
        assert!(result.is_some());
        assert_eq!(result.map(|r| r.content), Some("Hello world".to_string()));
    }

    #[test]
    fn test_miss() {
        let cache = ExactCache::new(100, Duration::from_secs(300));
        assert!(cache.get(99).is_none());
    }

    #[test]
    fn test_ttl_expiration() {
        let cache = ExactCache::new(100, Duration::from_millis(1));
        cache.insert(42, make_response("test"));

        // Wait for TTL to expire
        std::thread::sleep(Duration::from_millis(10));
        assert!(cache.get(42).is_none());
    }

    #[test]
    fn test_hit_count_increment() {
        let cache = ExactCache::new(100, Duration::from_secs(300));
        cache.insert(42, make_response("test"));

        cache.get(42);
        cache.get(42);
        cache.get(42);

        let top = cache.top_by_hit_count(10);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].hit_count, 3);
    }

    #[test]
    fn test_eviction_on_capacity() {
        let cache = ExactCache::new(2, Duration::from_secs(300));

        cache.insert(1, make_response("first"));
        cache.insert(2, make_response("second"));
        // This should trigger eviction
        cache.insert(3, make_response("third"));

        // Should have at most 2 entries
        assert!(cache.len() <= 2);
    }

    #[test]
    fn test_insert_respects_max_entries_under_contention() {
        // Verify that after many inserts, cache stays bounded
        let cache = ExactCache::new(10, Duration::from_secs(300));
        for i in 0..100u64 {
            cache.insert(i, make_response(&format!("entry-{}", i)));
        }
        // Cache should be close to max_entries (within margin for concurrent eviction)
        assert!(cache.len() <= 15, "Cache should be bounded, got {}", cache.len());
    }

    #[test]
    fn test_top_by_hit_count() {
        let cache = ExactCache::new(100, Duration::from_secs(300));

        cache.insert(1, make_response("low"));
        cache.insert(2, make_response("high"));
        cache.insert(3, make_response("mid"));

        // Hit entry 2 many times
        for _ in 0..10 {
            cache.get(2);
        }
        for _ in 0..5 {
            cache.get(3);
        }
        cache.get(1);

        let top = cache.top_by_hit_count(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].hash, 2);
        assert_eq!(top[1].hash, 3);
    }
}
