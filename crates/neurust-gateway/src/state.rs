//! Shared application state for the gateway.

use std::sync::Arc;

use crate::contracts::{EventSender, EventStore, NeuRustConfig, Provider};
use neurust_intel::cache::TieredCache;
use neurust_intel::store::InMemoryEventStore;

/// Shared application state passed to all route handlers via Axum's state extractor.
#[derive(Clone)]
pub struct AppState {
    /// Registered LLM providers.
    pub providers: Arc<Vec<Arc<dyn Provider>>>,
    /// Gateway configuration.
    pub config: Arc<NeuRustConfig>,
    /// Event bus sender for publishing gateway events.
    pub event_tx: EventSender,
    /// Tiered cache (exact + optional semantic).
    pub cache: Option<Arc<TieredCache>>,
    /// Event store for cost/metric persistence.
    pub event_store: Arc<dyn EventStore>,
}

impl AppState {
    /// Create a new `AppState`.
    pub fn new(
        providers: Vec<Arc<dyn Provider>>,
        config: NeuRustConfig,
        event_tx: EventSender,
    ) -> Self {
        Self {
            providers: Arc::new(providers),
            config: Arc::new(config),
            event_tx,
            cache: None,
            event_store: InMemoryEventStore::new_arc(),
        }
    }

    /// Set the tiered cache.
    pub fn with_cache(mut self, cache: Arc<TieredCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Set the event store.
    pub fn with_event_store(mut self, store: Arc<dyn EventStore>) -> Self {
        self.event_store = store;
        self
    }

    /// Find a provider that supports the given model name.
    /// Returns the first matching provider, respecting provider config priority.
    pub fn find_provider(&self, model: &str) -> Option<Arc<dyn Provider>> {
        // Build a priority map from config
        let priority_map: std::collections::HashMap<&str, u32> = self
            .config
            .providers
            .iter()
            .map(|p| (p.id.as_str(), p.priority.unwrap_or(u32::MAX)))
            .collect();

        let mut candidates: Vec<&Arc<dyn Provider>> = self
            .providers
            .iter()
            .filter(|p| p.supported_models().iter().any(|m| m == model))
            .collect();

        // Sort by priority (lower = higher priority)
        candidates.sort_by_key(|p| {
            priority_map.get(p.name()).copied().unwrap_or(u32::MAX)
        });

        candidates.into_iter().next().cloned()
    }

    /// List all models across all registered providers (deduplicated).
    pub fn all_models(&self) -> Vec<(String, String)> {
        let mut models = Vec::new();
        for provider in self.providers.iter() {
            for model in provider.supported_models() {
                models.push((model, provider.name().to_string()));
            }
        }
        models
    }
}
