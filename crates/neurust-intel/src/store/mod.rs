//! # Event Store
//!
//! Pluggable event storage backends. The `InMemoryEventStore` is always
//! available as the default. SQLite and PostgreSQL stores are feature-gated.

#[cfg(feature = "store-sqlite")]
pub mod sqlite_store;

#[cfg(feature = "store-postgres")]
pub mod postgres_store;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::contracts::{CostEvent, EventStore, MetricEntry};

/// In-memory event store backed by `Vec`.
///
/// Suitable for development, testing, and single-instance deployments
/// where persistence across restarts is not required.
pub struct InMemoryEventStore {
    cost_events: RwLock<Vec<CostEvent>>,
    metrics: RwLock<Vec<MetricEntry>>,
}

impl InMemoryEventStore {
    /// Create a new empty in-memory event store.
    pub fn new() -> Self {
        Self {
            cost_events: RwLock::new(Vec::new()),
            metrics: RwLock::new(Vec::new()),
        }
    }

    /// Create wrapped in an Arc for shared ownership.
    pub fn new_arc() -> Arc<Self> {
        Arc::new(Self::new())
    }

    /// Current number of stored cost events.
    pub async fn cost_event_count(&self) -> usize {
        self.cost_events.read().await.len()
    }

    /// Current number of stored metric entries.
    pub async fn metric_count(&self) -> usize {
        self.metrics.read().await.len()
    }

    /// Record a metric entry.
    pub async fn record_metric(&self, entry: MetricEntry) -> Result<()> {
        self.metrics.write().await.push(entry);
        Ok(())
    }

    /// Clear all stored data.
    pub async fn clear(&self) {
        self.cost_events.write().await.clear();
        self.metrics.write().await.clear();
    }
}

impl Default for InMemoryEventStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventStore for InMemoryEventStore {
    async fn record_cost(&self, event: &CostEvent) -> Result<()> {
        self.cost_events.write().await.push(event.clone());
        Ok(())
    }

    async fn daily_costs(&self, days: u32) -> Result<Vec<f64>> {
        let events = self.cost_events.read().await;

        if events.is_empty() {
            return Ok(Vec::new());
        }

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let day_ms: u64 = 86_400_000;
        let cutoff_ms = now_ms.saturating_sub(u64::from(days) * day_ms);

        // Group events by day
        let mut daily: HashMap<u64, f64> = HashMap::new();
        for event in events.iter() {
            if event.timestamp_ms >= cutoff_ms {
                let day_key = event.timestamp_ms / day_ms;
                *daily.entry(day_key).or_default() += event.cost_usd;
            }
        }

        // Sort by day and return costs
        let mut days_sorted: Vec<u64> = daily.keys().cloned().collect();
        days_sorted.sort();

        Ok(days_sorted.iter().map(|d| daily[d]).collect())
    }

    async fn all_cost_events(&self) -> Result<Vec<CostEvent>> {
        Ok(self.cost_events.read().await.clone())
    }

    async fn all_metrics(&self) -> Result<Vec<MetricEntry>> {
        Ok(self.metrics.read().await.clone())
    }

    async fn ping(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cost_event(cost: f64, timestamp_ms: u64) -> CostEvent {
        CostEvent {
            timestamp_ms,
            provider_id: "openai".to_string(),
            model: "gpt-4o".to_string(),
            prompt_tokens: 100,
            completion_tokens: 200,
            cost_usd: cost,
            latency_ms: 150,
            cached: false,
        }
    }

    #[tokio::test]
    async fn test_record_and_retrieve() {
        let store = InMemoryEventStore::new();
        let event = make_cost_event(0.05, 1000);

        store.record_cost(&event).await.expect("record");
        let events = store.all_cost_events().await.expect("get events");
        assert_eq!(events.len(), 1);
        assert!((events[0].cost_usd - 0.05).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_ping() {
        let store = InMemoryEventStore::new();
        assert!(store.ping().await.is_ok());
    }

    #[tokio::test]
    async fn test_clear() {
        let store = InMemoryEventStore::new();
        store
            .record_cost(&make_cost_event(0.01, 1000))
            .await
            .expect("record");
        assert_eq!(store.cost_event_count().await, 1);

        store.clear().await;
        assert_eq!(store.cost_event_count().await, 0);
    }

    #[tokio::test]
    async fn test_daily_costs_empty() {
        let store = InMemoryEventStore::new();
        let daily = store.daily_costs(30).await.expect("daily costs");
        assert!(daily.is_empty());
    }

    #[tokio::test]
    async fn test_record_metric() {
        let store = InMemoryEventStore::new();
        let metric = MetricEntry {
            timestamp_ms: 1000,
            metric_name: "cache_hit_rate".to_string(),
            value: 0.85,
            labels: HashMap::new(),
        };

        store.record_metric(metric).await.expect("record metric");
        let metrics = store.all_metrics().await.expect("get metrics");
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].metric_name, "cache_hit_rate");
    }

    #[tokio::test]
    async fn test_new_arc() {
        let store = InMemoryEventStore::new_arc();
        assert!(store.ping().await.is_ok());
    }
}
