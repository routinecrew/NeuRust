//! # LFU Eviction
//!
//! Least-Frequently-Used eviction strategy for ExactCache.
//! When cache exceeds capacity, removes entries with the lowest hit count.

use std::sync::atomic::Ordering;

use dashmap::DashMap;

use super::exact_cache::CacheEntry;

/// Evict the least-frequently-used entry from the cache.
///
/// Scans all entries and removes the one with the lowest hit count.
/// Also removes expired entries encountered during the scan.
pub fn evict_lfu(map: &DashMap<u64, CacheEntry>) {
    // First pass: remove all expired entries
    let expired_keys: Vec<u64> = map
        .iter()
        .filter(|e| e.value().is_expired())
        .map(|e| *e.key())
        .collect();

    for key in &expired_keys {
        map.remove(key);
    }

    // If we freed enough space by removing expired entries, we're done
    if !expired_keys.is_empty() {
        return;
    }

    // Second pass: find the entry with the lowest hit count
    let victim = map
        .iter()
        .min_by_key(|e| e.value().hit_count.load(Ordering::Relaxed))
        .map(|e| *e.key());

    if let Some(key) = victim {
        map.remove(&key);
        tracing::trace!(key = key, "Evicted LFU cache entry");
    }
}

/// Evict multiple entries to free up space, targeting the N least-used.
pub fn evict_lfu_batch(map: &DashMap<u64, CacheEntry>, count: usize) {
    // Remove expired first
    let expired_keys: Vec<u64> = map
        .iter()
        .filter(|e| e.value().is_expired())
        .map(|e| *e.key())
        .collect();

    let mut removed = 0;
    for key in expired_keys {
        map.remove(&key);
        removed += 1;
        if removed >= count {
            return;
        }
    }

    // If still need to evict more, sort by hit count and remove lowest
    let remaining = count - removed;
    let mut entries: Vec<(u64, u64)> = map
        .iter()
        .map(|e| (*e.key(), e.value().hit_count.load(Ordering::Relaxed)))
        .collect();

    entries.sort_by_key(|(_, hits)| *hits);

    for (key, _) in entries.into_iter().take(remaining) {
        map.remove(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::*;
    use std::sync::atomic::AtomicU64;
    use std::time::{Duration, Instant};

    fn make_entry(hit_count: u64, ttl_secs: u64) -> CacheEntry {
        CacheEntry {
            response: UnifiedResponse {
                content: "test".to_string(),
                model: "gpt-4o".to_string(),
                usage: TokenUsage::default(),
                provider_id: "openai".to_string(),
                latency_ms: 100,
                upstream_id: None,
            },
            hit_count: AtomicU64::new(hit_count),
            created_at: Instant::now(),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    #[test]
    fn test_evict_lfu_removes_lowest_hit() {
        let map = DashMap::new();
        map.insert(1, make_entry(10, 300));
        map.insert(2, make_entry(1, 300)); // lowest
        map.insert(3, make_entry(5, 300));

        evict_lfu(&map);

        assert_eq!(map.len(), 2);
        assert!(map.get(&2).is_none()); // lowest hit count removed
    }

    #[test]
    fn test_evict_lfu_prefers_expired() {
        let map = DashMap::new();
        map.insert(1, make_entry(10, 300));
        map.insert(2, make_entry(100, 0)); // expired (0s TTL)

        // Wait a tiny bit so the 0-TTL entry expires
        std::thread::sleep(Duration::from_millis(5));

        evict_lfu(&map);

        assert_eq!(map.len(), 1);
        assert!(map.get(&1).is_some()); // high-hit entry kept
    }

    #[test]
    fn test_evict_batch() {
        let map = DashMap::new();
        map.insert(1, make_entry(1, 300));
        map.insert(2, make_entry(2, 300));
        map.insert(3, make_entry(3, 300));
        map.insert(4, make_entry(4, 300));

        evict_lfu_batch(&map, 2);

        assert_eq!(map.len(), 2);
        // Entries 3 and 4 should remain (highest hit counts)
        assert!(map.get(&3).is_some());
        assert!(map.get(&4).is_some());
    }
}
