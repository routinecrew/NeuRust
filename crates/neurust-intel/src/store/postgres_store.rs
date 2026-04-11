//! # PostgreSQL Event Store
//!
//! Persistent event storage backed by PostgreSQL. Feature-gated under `store-postgres`.
//! Uses sqlx with async connection pooling for production-grade deployments.

use std::collections::HashMap;

use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};

use crate::contracts::{CostEvent, EventStore, MetricEntry};

/// PostgreSQL-backed event store.
///
/// Uses `sqlx::PgPool` for async connection pooling. Suitable for
/// multi-instance deployments where shared persistent storage is required.
pub struct PostgresEventStore {
    pool: PgPool,
}

impl PostgresEventStore {
    /// Connect to a PostgreSQL database at the given URL.
    ///
    /// Creates a connection pool and runs migrations automatically.
    pub async fn new(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await
            .with_context(|| format!("Failed to connect to PostgreSQL at {}", database_url))?;

        let store = Self { pool };
        store.migrate().await?;

        tracing::info!(url = database_url, "PostgreSQL event store initialized");
        Ok(store)
    }

    /// Create a store from an existing connection pool.
    ///
    /// Runs migrations automatically.
    pub async fn from_pool(pool: PgPool) -> Result<Self> {
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    /// Run schema migrations.
    async fn migrate(&self) -> Result<()> {
        sqlx::query(
            "
            CREATE TABLE IF NOT EXISTS cost_events (
                id BIGSERIAL PRIMARY KEY,
                timestamp_ms BIGINT NOT NULL,
                provider_id TEXT NOT NULL,
                model TEXT NOT NULL,
                prompt_tokens INTEGER NOT NULL,
                completion_tokens INTEGER NOT NULL,
                cost_usd DOUBLE PRECISION NOT NULL,
                latency_ms BIGINT NOT NULL,
                cached BOOLEAN NOT NULL DEFAULT FALSE
            )
            ",
        )
        .execute(&self.pool)
        .await
        .context("Failed to create cost_events table")?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_cost_events_timestamp ON cost_events(timestamp_ms)",
        )
        .execute(&self.pool)
        .await
        .context("Failed to create timestamp index")?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_cost_events_provider ON cost_events(provider_id, model)",
        )
        .execute(&self.pool)
        .await
        .context("Failed to create provider index")?;

        sqlx::query(
            "
            CREATE TABLE IF NOT EXISTS metrics (
                id BIGSERIAL PRIMARY KEY,
                timestamp_ms BIGINT NOT NULL,
                metric_name TEXT NOT NULL,
                value DOUBLE PRECISION NOT NULL,
                labels JSONB NOT NULL DEFAULT '{}'
            )
            ",
        )
        .execute(&self.pool)
        .await
        .context("Failed to create metrics table")?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_metrics_timestamp ON metrics(timestamp_ms)",
        )
        .execute(&self.pool)
        .await
        .context("Failed to create metrics timestamp index")?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_metrics_name ON metrics(metric_name)",
        )
        .execute(&self.pool)
        .await
        .context("Failed to create metrics name index")?;

        Ok(())
    }
}

#[async_trait]
impl EventStore for PostgresEventStore {
    async fn record_cost(&self, event: &CostEvent) -> Result<()> {
        sqlx::query(
            "INSERT INTO cost_events (timestamp_ms, provider_id, model, prompt_tokens, completion_tokens, cost_usd, latency_ms, cached)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(event.timestamp_ms as i64)
        .bind(&event.provider_id)
        .bind(&event.model)
        .bind(event.prompt_tokens as i32)
        .bind(event.completion_tokens as i32)
        .bind(event.cost_usd)
        .bind(event.latency_ms as i64)
        .bind(event.cached)
        .execute(&self.pool)
        .await
        .context("Failed to insert cost event")?;

        tracing::trace!(
            model = %event.model,
            cost = event.cost_usd,
            "Cost event persisted to PostgreSQL"
        );
        Ok(())
    }

    async fn daily_costs(&self, days: u32) -> Result<Vec<f64>> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let day_ms: i64 = 86_400_000;
        let cutoff_ms = now_ms.saturating_sub(i64::from(days) * day_ms);

        let rows = sqlx::query(
            "SELECT (timestamp_ms / $1) AS day_key, SUM(cost_usd) AS total_cost
             FROM cost_events
             WHERE timestamp_ms >= $2
             GROUP BY day_key
             ORDER BY day_key ASC",
        )
        .bind(day_ms)
        .bind(cutoff_ms)
        .fetch_all(&self.pool)
        .await
        .context("Failed to query daily costs")?;

        let costs: Vec<f64> = rows
            .iter()
            .map(|row| row.get::<f64, _>("total_cost"))
            .collect();

        Ok(costs)
    }

    async fn all_cost_events(&self) -> Result<Vec<CostEvent>> {
        let rows = sqlx::query(
            "SELECT timestamp_ms, provider_id, model, prompt_tokens, completion_tokens, cost_usd, latency_ms, cached
             FROM cost_events ORDER BY timestamp_ms ASC",
        )
        .fetch_all(&self.pool)
        .await
        .context("Failed to query cost events")?;

        let events = rows
            .iter()
            .map(|row| CostEvent {
                timestamp_ms: row.get::<i64, _>("timestamp_ms") as u64,
                provider_id: row.get("provider_id"),
                model: row.get("model"),
                prompt_tokens: row.get::<i32, _>("prompt_tokens") as u32,
                completion_tokens: row.get::<i32, _>("completion_tokens") as u32,
                cost_usd: row.get("cost_usd"),
                latency_ms: row.get::<i64, _>("latency_ms") as u64,
                cached: row.get("cached"),
            })
            .collect();

        Ok(events)
    }

    async fn all_metrics(&self) -> Result<Vec<MetricEntry>> {
        let rows = sqlx::query(
            "SELECT timestamp_ms, metric_name, value, labels FROM metrics ORDER BY timestamp_ms ASC",
        )
        .fetch_all(&self.pool)
        .await
        .context("Failed to query metrics")?;

        let metrics = rows
            .iter()
            .map(|row| {
                let labels_json: serde_json::Value = row.get("labels");
                let labels: HashMap<String, String> =
                    serde_json::from_value(labels_json).unwrap_or_default();
                MetricEntry {
                    timestamp_ms: row.get::<i64, _>("timestamp_ms") as u64,
                    metric_name: row.get("metric_name"),
                    value: row.get("value"),
                    labels,
                }
            })
            .collect();

        Ok(metrics)
    }

    async fn ping(&self) -> Result<()> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .context("PostgreSQL ping failed")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cost_event(cost: f64, ts: u64) -> CostEvent {
        CostEvent {
            timestamp_ms: ts,
            provider_id: "openai".to_string(),
            model: "gpt-4o".to_string(),
            prompt_tokens: 100,
            completion_tokens: 200,
            cost_usd: cost,
            latency_ms: 150,
            cached: false,
        }
    }

    /// Helper to get a test database URL from the environment.
    /// Tests are skipped if `TEST_POSTGRES_URL` is not set.
    fn test_database_url() -> Option<String> {
        std::env::var("TEST_POSTGRES_URL").ok()
    }

    #[tokio::test]
    async fn test_postgres_new_requires_valid_url() {
        // Connecting to an invalid URL should fail gracefully
        let result = PostgresEventStore::new("postgres://invalid:5432/nonexistent").await;
        assert!(result.is_err(), "Should fail with invalid connection URL");
    }

    #[tokio::test]
    async fn test_postgres_record_and_retrieve() {
        let Some(url) = test_database_url() else {
            eprintln!("Skipping: TEST_POSTGRES_URL not set");
            return;
        };

        let store = PostgresEventStore::new(&url).await.expect("connect");
        let event = make_cost_event(0.05, 1000);

        store.record_cost(&event).await.expect("record");

        let events = store.all_cost_events().await.expect("get events");
        assert!(!events.is_empty());

        let last = events.last().expect("at least one event");
        assert!((last.cost_usd - 0.05).abs() < f64::EPSILON);
        assert_eq!(last.provider_id, "openai");
        assert_eq!(last.model, "gpt-4o");
    }

    #[tokio::test]
    async fn test_postgres_daily_costs() {
        let Some(url) = test_database_url() else {
            eprintln!("Skipping: TEST_POSTGRES_URL not set");
            return;
        };

        let store = PostgresEventStore::new(&url).await.expect("connect");

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        store
            .record_cost(&make_cost_event(0.10, now_ms - 1000))
            .await
            .expect("record");
        store
            .record_cost(&make_cost_event(0.20, now_ms - 500))
            .await
            .expect("record");

        let daily = store.daily_costs(7).await.expect("daily costs");
        assert!(!daily.is_empty(), "Should have at least one day of costs");
    }

    #[tokio::test]
    async fn test_postgres_ping() {
        let Some(url) = test_database_url() else {
            eprintln!("Skipping: TEST_POSTGRES_URL not set");
            return;
        };

        let store = PostgresEventStore::new(&url).await.expect("connect");
        assert!(store.ping().await.is_ok());
    }

    #[tokio::test]
    async fn test_postgres_cached_flag() {
        let Some(url) = test_database_url() else {
            eprintln!("Skipping: TEST_POSTGRES_URL not set");
            return;
        };

        let store = PostgresEventStore::new(&url).await.expect("connect");

        let mut event = make_cost_event(0.01, 1000);
        event.cached = true;
        store.record_cost(&event).await.expect("record");

        let events = store.all_cost_events().await.expect("get events");
        let cached_event = events.iter().find(|e| e.timestamp_ms == 1000 && e.cached);
        assert!(cached_event.is_some(), "Cached flag should be preserved");
    }
}
