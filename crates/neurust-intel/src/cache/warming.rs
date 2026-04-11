//! # Cache Warming (v3)
//!
//! Prevents cold start after server restart by periodically dumping
//! the top-N cache entries to disk (bincode) and loading them on startup.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tracing;

use super::exact_cache::{CacheDumpEntry, ExactCache};

/// Cache warmer: dumps hot entries to disk and loads them on startup.
pub struct CacheWarmer {
    /// Path to the dump file.
    dump_path: PathBuf,
    /// Interval between periodic dumps.
    dump_interval: Duration,
    /// Maximum entries to warm on startup.
    max_warm_entries: usize,
}

impl CacheWarmer {
    /// Create a new cache warmer.
    pub fn new(dump_path: PathBuf, dump_interval: Duration, max_warm_entries: usize) -> Self {
        Self {
            dump_path,
            dump_interval,
            max_warm_entries,
        }
    }

    /// Create from config values with defaults.
    pub fn from_config(
        dump_path: Option<&str>,
        dump_interval_sec: Option<u64>,
        max_warm_entries: Option<usize>,
    ) -> Self {
        Self {
            dump_path: PathBuf::from(dump_path.unwrap_or("./cache_dump.bin")),
            dump_interval: Duration::from_secs(dump_interval_sec.unwrap_or(300)),
            max_warm_entries: max_warm_entries.unwrap_or(10_000),
        }
    }

    /// Load cached entries from disk on server startup.
    ///
    /// Returns the number of entries loaded. Returns Ok(0) if the dump
    /// file does not exist (first startup).
    pub async fn warm(&self, cache: &ExactCache) -> Result<usize> {
        if !self.dump_path.exists() {
            tracing::info!(
                path = %self.dump_path.display(),
                "No cache dump file found, starting with empty cache"
            );
            return Ok(0);
        }

        let data = tokio::fs::read(&self.dump_path).await?;
        let entries: Vec<CacheDumpEntry> = bincode::deserialize(&data)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize cache dump: {}", e))?;

        let loaded = entries
            .into_iter()
            .take(self.max_warm_entries)
            .map(|e| {
                cache.insert_warm(e.hash, e.response, e.hit_count);
            })
            .count();

        tracing::info!(
            entries = loaded,
            path = %self.dump_path.display(),
            "Cache warmed from dump file"
        );
        Ok(loaded)
    }

    /// Periodically dump top-N entries to disk.
    ///
    /// Respects the shutdown signal for graceful termination.
    /// Writes to a temp file then atomically renames to prevent corruption.
    pub async fn dump_loop(
        &self,
        cache: Arc<ExactCache>,
        mut shutdown: tokio::sync::watch::Receiver<()>,
    ) {
        let mut interval = tokio::time::interval(self.dump_interval);
        let mut consecutive_failures = 0u32;

        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = shutdown.changed() => {
                    tracing::info!("CacheWarmer shutting down, performing final dump");
                    self.dump_once(&cache).await;
                    return;
                }
            }

            if self.dump_once(&cache).await {
                consecutive_failures = 0;
            } else {
                consecutive_failures += 1;
                if consecutive_failures >= 5 {
                    tracing::error!(
                        failures = consecutive_failures,
                        "CacheWarmer: too many consecutive failures, backing off"
                    );
                    tokio::time::sleep(self.dump_interval * 2).await;
                }
            }
        }
    }

    /// Perform a single cache dump. Returns true on success.
    async fn dump_once(&self, cache: &ExactCache) -> bool {
        let top_entries = cache.top_by_hit_count(self.max_warm_entries);
        if top_entries.is_empty() {
            return true;
        }

        match bincode::serialize(&top_entries) {
            Ok(bytes) => {
                // Atomic write: write to temp file, then rename
                let tmp_path = self.dump_path.with_extension("tmp");
                if let Err(e) = tokio::fs::write(&tmp_path, bytes).await {
                    tracing::warn!(error = %e, path = %self.dump_path.display(), "Failed to write cache dump");
                    return false;
                }
                if let Err(e) = tokio::fs::rename(&tmp_path, &self.dump_path).await {
                    tracing::warn!(error = %e, "Failed to rename cache dump");
                    return false;
                }
                tracing::debug!(entries = top_entries.len(), path = %self.dump_path.display(), "Cache dump written");
                true
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to serialize cache dump");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::*;
    use std::time::Duration;

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

    #[tokio::test]
    async fn test_warm_no_file() {
        let warmer = CacheWarmer::new(
            PathBuf::from("/tmp/neurust_test_nonexistent.bin"),
            Duration::from_secs(60),
            100,
        );
        let cache = ExactCache::new(100, Duration::from_secs(300));
        let loaded = warmer.warm(&cache).await.expect("warm should succeed");
        assert_eq!(loaded, 0);
    }

    #[tokio::test]
    async fn test_dump_and_warm_roundtrip() {
        let dump_path = std::env::temp_dir().join("neurust_test_cache_dump.bin");

        // Clean up from previous test runs
        let _ = tokio::fs::remove_file(&dump_path).await;

        let warmer = CacheWarmer::new(dump_path.clone(), Duration::from_secs(60), 100);

        // Create and populate a cache
        let cache = Arc::new(ExactCache::new(100, Duration::from_secs(300)));
        cache.insert(1, make_response("first"));
        cache.insert(2, make_response("second"));

        // Hit entry 1 a few times
        cache.get(1);
        cache.get(1);

        // Dump to disk
        let top = cache.top_by_hit_count(100);
        let bytes = bincode::serialize(&top).expect("serialize");
        tokio::fs::write(&dump_path, bytes)
            .await
            .expect("write dump");

        // Create a fresh cache and warm it
        let fresh_cache = ExactCache::new(100, Duration::from_secs(300));
        let loaded = warmer.warm(&fresh_cache).await.expect("warm");
        assert_eq!(loaded, 2);
        assert_eq!(fresh_cache.len(), 2);

        // Verify content
        let r = fresh_cache.get(1);
        assert!(r.is_some());
        assert_eq!(r.map(|r| r.content), Some("first".to_string()));

        // Clean up
        let _ = tokio::fs::remove_file(&dump_path).await;
    }
}
