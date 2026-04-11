//! # Semantic Cache
//!
//! Embedding-based similarity cache. Uses fastembed for embedding generation
//! and cosine similarity for matching. Feature-gated under `cache-semantic`.
//!
//! Lookup flow: compute embedding for the query, scan stored embeddings,
//! return the best match above the similarity threshold.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use dashmap::DashMap;
use tracing;

use crate::contracts::UnifiedResponse;

/// A cached entry with its embedding vector.
struct SemanticEntry {
    /// The embedding vector for this entry's query.
    embedding: Vec<f32>,
    /// The cached response.
    response: UnifiedResponse,
    /// Hit count for LFU eviction.
    hit_count: AtomicU64,
    /// When this entry was created.
    created_at: Instant,
    /// Time-to-live.
    ttl: Duration,
}

impl SemanticEntry {
    fn is_expired(&self) -> bool {
        self.created_at.elapsed() > self.ttl
    }
}

/// Semantic similarity cache backed by fastembed embeddings.
///
/// Uses a brute-force linear scan for similarity search. For production
/// workloads exceeding ~100K entries, consider adding an ANN index.
pub struct SemanticCache {
    entries: DashMap<u64, SemanticEntry>,
    model: Mutex<fastembed::TextEmbedding>,
    similarity_threshold: f32,
    max_entries: usize,
    default_ttl: Duration,
    next_id: AtomicU64,
}

impl SemanticCache {
    /// Create a new semantic cache.
    ///
    /// Initializes the fastembed model on first call. This may take a few
    /// seconds as the model weights are loaded.
    pub fn new(
        max_entries: usize,
        default_ttl: Duration,
        similarity_threshold: f32,
    ) -> Result<Self> {
        let options = fastembed::TextInitOptions::new(fastembed::EmbeddingModel::AllMiniLML6V2);
        let model = fastembed::TextEmbedding::try_new(options)
            .context("Failed to initialize fastembed model")?;

        Ok(Self {
            entries: DashMap::with_capacity(max_entries),
            model: Mutex::new(model),
            similarity_threshold,
            max_entries,
            default_ttl,
            next_id: AtomicU64::new(0),
        })
    }

    /// Look up the most similar cached response for the given query text.
    ///
    /// Returns `None` if no entry exceeds the similarity threshold.
    pub fn lookup(&self, query: &str) -> Option<UnifiedResponse> {
        let query_embedding = match self.embed(query) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to compute query embedding");
                return None;
            }
        };

        let mut best_score: f32 = 0.0;
        let mut best_response: Option<UnifiedResponse> = None;
        let mut expired_keys = Vec::new();

        for entry in self.entries.iter() {
            if entry.value().is_expired() {
                expired_keys.push(*entry.key());
                continue;
            }

            let score = cosine_similarity(&query_embedding, &entry.value().embedding);
            if score > best_score && score >= self.similarity_threshold {
                best_score = score;
                best_response = Some(entry.value().response.clone());
                entry.value().hit_count.fetch_add(1, Ordering::Relaxed);
            }
        }

        // Clean up expired entries
        for key in expired_keys {
            self.entries.remove(&key);
        }

        if best_response.is_some() {
            tracing::debug!(score = best_score, "Semantic cache hit");
        }

        best_response
    }

    /// Store a query-response pair in the cache.
    pub fn store(&self, query: &str, response: &UnifiedResponse) {
        let embedding = match self.embed(query) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to compute embedding for cache store");
                return;
            }
        };

        self.evict_if_needed();

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.entries.insert(
            id,
            SemanticEntry {
                embedding,
                response: response.clone(),
                hit_count: AtomicU64::new(0),
                created_at: Instant::now(),
                ttl: self.default_ttl,
            },
        );
    }

    /// Current number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Compute embedding for a text string.
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut model = self
            .model
            .lock()
            .map_err(|e| anyhow::anyhow!("Embedding model lock poisoned: {}", e))?;

        let embeddings = model
            .embed(vec![text], None)
            .context("Embedding generation failed")?;

        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Empty embedding result"))
    }

    /// Evict LFU entries if at capacity.
    fn evict_if_needed(&self) {
        if self.entries.len() < self.max_entries {
            return;
        }

        // Remove expired first
        let expired: Vec<u64> = self
            .entries
            .iter()
            .filter(|e| e.value().is_expired())
            .map(|e| *e.key())
            .collect();

        for key in &expired {
            self.entries.remove(key);
        }

        if self.entries.len() < self.max_entries {
            return;
        }

        // LFU: remove entry with lowest hit count
        if let Some(victim) = self
            .entries
            .iter()
            .min_by_key(|e| e.value().hit_count.load(Ordering::Relaxed))
            .map(|e| *e.key())
        {
            self.entries.remove(&victim);
        }
    }
}

/// Cosine similarity between two vectors.
///
/// Uses simsimd for SIMD-accelerated computation when available,
/// falls back to scalar computation.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    // Use simsimd for SIMD-accelerated cosine similarity
    match simsimd::SpatialSimilarity::cosine(a, b) {
        Some(distance) => {
            // simsimd returns cosine distance (1 - similarity), convert to similarity
            (1.0 - distance as f32).max(0.0)
        }
        None => scalar_cosine(a, b),
    }
}

/// Scalar fallback for cosine similarity.
fn scalar_cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a < f32::EPSILON || norm_b < f32::EPSILON {
        return 0.0;
    }
    (dot / (norm_a * norm_b)).clamp(0.0, 1.0)
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
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            (sim - 1.0).abs() < 0.01,
            "Identical vectors should have similarity ~1.0, got {}",
            sim
        );
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            sim.abs() < 0.01,
            "Orthogonal vectors should have similarity ~0.0, got {}",
            sim
        );
    }

    #[test]
    fn test_cosine_similarity_empty() {
        let sim = cosine_similarity(&[], &[]);
        assert!((sim - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_cosine_similarity_mismatched_length() {
        let sim = cosine_similarity(&[1.0, 2.0], &[1.0]);
        assert!((sim - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_scalar_cosine_similarity() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = scalar_cosine(&a, &b);
        assert!(
            (sim - 1.0).abs() < 0.01,
            "Identical vectors should have scalar similarity ~1.0, got {}",
            sim
        );
    }

    #[test]
    fn test_semantic_cache_creation() {
        let cache = SemanticCache::new(100, Duration::from_secs(300), 0.9);
        assert!(
            cache.is_ok(),
            "SemanticCache should initialize successfully"
        );
        assert!(cache.unwrap().is_empty());
    }

    #[test]
    fn test_semantic_cache_store_and_lookup_similar() {
        let cache =
            SemanticCache::new(100, Duration::from_secs(300), 0.7).expect("cache init");

        let resp = make_response("The capital of France is Paris");
        cache.store("What is the capital of France?", &resp);

        // Very similar query should hit
        let result = cache.lookup("What's the capital of France?");
        assert!(result.is_some(), "Similar query should hit semantic cache");
    }

    #[test]
    fn test_semantic_cache_store_and_lookup_dissimilar() {
        let cache =
            SemanticCache::new(100, Duration::from_secs(300), 0.9).expect("cache init");

        let resp = make_response("The capital of France is Paris");
        cache.store("What is the capital of France?", &resp);

        // Very different query should miss with high threshold
        let result = cache.lookup("How do I cook pasta?");
        assert!(
            result.is_none(),
            "Dissimilar query should miss semantic cache"
        );
    }

    #[test]
    fn test_semantic_cache_eviction() {
        let cache =
            SemanticCache::new(2, Duration::from_secs(300), 0.8).expect("cache init");

        cache.store("query one", &make_response("resp 1"));
        cache.store("query two", &make_response("resp 2"));
        cache.store("query three", &make_response("resp 3"));

        assert!(
            cache.len() <= 2,
            "Cache should evict to stay at max_entries"
        );
    }
}
