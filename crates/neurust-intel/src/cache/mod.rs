//! # Tiered Cache
//!
//! Two-tier caching: ExactCache (hash-based, always available) and optional
//! SemanticCache (embedding-based, feature-gated). Implements `PipelineLayer`
//! for transparent integration into the request pipeline.

pub mod cache_key;
pub mod exact_cache;
pub mod eviction;
pub mod warming;

#[cfg(feature = "cache-semantic")]
pub mod semantic_cache;

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tracing;

use crate::contracts::{PipelineLayer, UnifiedRequest, UnifiedResponse};
use cache_key::compute_cache_key;
use exact_cache::ExactCache;
use warming::CacheWarmer;

#[cfg(feature = "cache-semantic")]
use semantic_cache::SemanticCache;

/// Two-tier cache: exact hash lookup, then optional semantic similarity.
pub struct TieredCache {
    exact: Arc<ExactCache>,
    #[cfg(feature = "cache-semantic")]
    semantic: Option<Arc<SemanticCache>>,
    warmer: Option<CacheWarmer>,
}

impl TieredCache {
    /// Create a new tiered cache with the given exact cache and optional warmer.
    pub fn new(exact: Arc<ExactCache>, warmer: Option<CacheWarmer>) -> Self {
        Self {
            exact,
            #[cfg(feature = "cache-semantic")]
            semantic: None,
            warmer,
        }
    }

    /// Create a tiered cache with both exact and semantic layers.
    #[cfg(feature = "cache-semantic")]
    pub fn with_semantic(
        exact: Arc<ExactCache>,
        semantic: Arc<SemanticCache>,
        warmer: Option<CacheWarmer>,
    ) -> Self {
        Self {
            exact,
            semantic: Some(semantic),
            warmer,
        }
    }

    /// Look up a cached response for the given request.
    ///
    /// Checks exact cache first, then falls back to semantic cache.
    /// Returns `None` on miss.
    pub fn lookup(&self, request: &UnifiedRequest) -> Option<UnifiedResponse> {
        let hash = compute_cache_key(request);

        // Tier 1: exact match
        if let Some(response) = self.exact.get(hash) {
            tracing::debug!(hash = hash, tier = "exact", "Cache hit");
            return Some(response);
        }

        // Tier 2: semantic similarity (if enabled)
        #[cfg(feature = "cache-semantic")]
        if let Some(ref semantic) = self.semantic {
            let query = request.last_user_message();
            if let Some(response) = semantic.lookup(&query) {
                tracing::debug!(tier = "semantic", "Cache hit");
                // Promote to exact cache for faster future lookups
                self.exact.insert(hash, response.clone());
                return Some(response);
            }
        }

        None
    }

    /// Store a response in the cache.
    pub fn store(&self, request: &UnifiedRequest, response: &UnifiedResponse) {
        let hash = compute_cache_key(request);
        self.exact.insert(hash, response.clone());

        #[cfg(feature = "cache-semantic")]
        if let Some(ref semantic) = self.semantic {
            let query = request.last_user_message();
            semantic.store(&query, response);
        }

        tracing::debug!(hash = hash, "Cached response");
    }

    /// Warm the cache from a dump file on startup.
    pub async fn warm(&self) -> Result<usize> {
        match &self.warmer {
            Some(w) => w.warm(&self.exact).await,
            None => Ok(0),
        }
    }

    /// Start the periodic dump loop (background task).
    /// Pass a shutdown receiver to allow graceful termination.
    pub async fn start_dump_loop(&self, shutdown: tokio::sync::watch::Receiver<()>) {
        if let Some(warmer) = &self.warmer {
            warmer.dump_loop(Arc::clone(&self.exact), shutdown).await;
        }
    }

    /// Get a reference to the underlying exact cache.
    pub fn exact_cache(&self) -> &Arc<ExactCache> {
        &self.exact
    }
}

#[async_trait]
impl PipelineLayer for TieredCache {
    fn name(&self) -> &str {
        "tiered-cache"
    }

    async fn on_request(&self, request: &mut UnifiedRequest) -> Result<()> {
        // Cache lookup happens externally (pipeline checks for cached response).
        // The pipeline layer itself is a no-op on request phase since we cannot
        // short-circuit the pipeline from within on_request.
        let _ = request;
        Ok(())
    }

    async fn on_response(
        &self,
        request: &UnifiedRequest,
        response: &mut UnifiedResponse,
    ) -> Result<()> {
        self.store(request, response);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::*;
    use std::collections::HashMap;
    use std::time::Duration;

    fn make_request(user_msg: &str) -> UnifiedRequest {
        UnifiedRequest {
            model: ModelSpec {
                model_name: "gpt-4o".to_string(),
                provider_id: None,
            },
            messages: vec![Message {
                role: Role::User,
                content: user_msg.to_string(),
            }],
            temperature: Some(0.7),
            max_tokens: Some(100),
            stream: false,
            context: RequestContext::default(),
            extra_params: HashMap::new(),
        }
    }

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
    fn test_tiered_cache_store_and_lookup() {
        let exact = Arc::new(ExactCache::new(1000, Duration::from_secs(300)));
        let cache = TieredCache::new(exact, None);

        let req = make_request("Hello");
        let resp = make_response("Hi there");

        assert!(cache.lookup(&req).is_none());
        cache.store(&req, &resp);
        let cached = cache.lookup(&req);
        assert!(cached.is_some());
        assert_eq!(cached.as_ref().map(|r| r.content.as_str()), Some("Hi there"));
    }

    #[test]
    fn test_different_requests_different_cache() {
        let exact = Arc::new(ExactCache::new(1000, Duration::from_secs(300)));
        let cache = TieredCache::new(exact, None);

        let req1 = make_request("Hello");
        let req2 = make_request("Goodbye");
        let resp = make_response("response");

        cache.store(&req1, &resp);
        assert!(cache.lookup(&req1).is_some());
        assert!(cache.lookup(&req2).is_none());
    }
}
