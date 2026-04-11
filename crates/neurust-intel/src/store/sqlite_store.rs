//! # SQLite Event Store
//!
//! Persistent event storage backed by SQLite. Feature-gated under `store-sqlite`.
//! Uses rusqlite with bundled SQLite for zero-dependency deployment.

use std::collections::HashMap;

use anyhow::{Context, Result};
use async_trait::async_trait;
use rusqlite::Connection;
use tokio::sync::Mutex;
use tracing;

use crate::contracts::{CostEvent, EventStore, MetricEntry};

/// SQLite-backed event store.
///
/// All operations are serialized through a Mutex since rusqlite's Connection
/// is not Send+Sync. For high-throughput scenarios, consider batching writes.
pub struct SqliteEventStore {
    conn: Mutex<Connection>,
}

impl SqliteEventStore {
    /// Open or create a SQLite database at the given path.
    ///
    /// Runs migrations automatically on first open.
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open SQLite database at {}", path))?;

        // Enable WAL mode for better concurrent read performance
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;

        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate_sync()?;

        tracing::info!(path = path, "SQLite event store initialized");
        Ok(store)
    }

    /// Open an in-memory SQLite database (for testing).
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .context("Failed to open in-memory SQLite database")?;

        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate_sync()?;
        Ok(store)
    }

    /// Run schema migrations.
    fn migrate_sync(&self) -> Result<()> {
        // We need to block here since this is called from non-async context
        // The Mutex is not yet contended since this runs during initialization.
        let conn = self.conn.try_lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire lock during migration"))?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS cost_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp_ms INTEGER NOT NULL,
                provider_id TEXT NOT NULL,
                model TEXT NOT NULL,
                prompt_tokens INTEGER NOT NULL,
                completion_tokens INTEGER NOT NULL,
                cost_usd REAL NOT NULL,
                latency_ms INTEGER NOT NULL,
                cached INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_cost_events_timestamp
                ON cost_events(timestamp_ms);

            CREATE INDEX IF NOT EXISTS idx_cost_events_provider
                ON cost_events(provider_id, model);

            CREATE TABLE IF NOT EXISTS metrics (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp_ms INTEGER NOT NULL,
                metric_name TEXT NOT NULL,
                value REAL NOT NULL,
                labels TEXT NOT NULL DEFAULT '{}'
            );

            CREATE INDEX IF NOT EXISTS idx_metrics_timestamp
                ON metrics(timestamp_ms);

            CREATE INDEX IF NOT EXISTS idx_metrics_name
                ON metrics(metric_name);
            ",
        )
        .context("SQLite migration failed")?;

        Ok(())
    }
}

#[async_trait]
impl EventStore for SqliteEventStore {
    async fn record_cost(&self, event: &CostEvent) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO cost_events (timestamp_ms, provider_id, model, prompt_tokens, completion_tokens, cost_usd, latency_ms, cached)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                event.timestamp_ms,
                event.provider_id,
                event.model,
                event.prompt_tokens,
                event.completion_tokens,
                event.cost_usd,
                event.latency_ms,
                event.cached as i32,
            ],
        )
        .context("Failed to insert cost event")?;

        tracing::trace!(
            model = %event.model,
            cost = event.cost_usd,
            "Cost event persisted to SQLite"
        );
        Ok(())
    }

    async fn daily_costs(&self, days: u32) -> Result<Vec<f64>> {
        let conn = self.conn.lock().await;

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let day_ms: u64 = 86_400_000;
        let cutoff_ms = now_ms.saturating_sub(u64::from(days) * day_ms);

        let mut stmt = conn.prepare(
            "SELECT (timestamp_ms / ?1) AS day_key, SUM(cost_usd)
             FROM cost_events
             WHERE timestamp_ms >= ?2
             GROUP BY day_key
             ORDER BY day_key ASC",
        )?;

        let rows = stmt
            .query_map(rusqlite::params![day_ms, cutoff_ms], |row| {
                row.get::<_, f64>(1)
            })?
            .collect::<std::result::Result<Vec<f64>, _>>()
            .context("Failed to query daily costs")?;

        Ok(rows)
    }

    async fn all_cost_events(&self) -> Result<Vec<CostEvent>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT timestamp_ms, provider_id, model, prompt_tokens, completion_tokens, cost_usd, latency_ms, cached
             FROM cost_events ORDER BY timestamp_ms ASC",
        )?;

        let events = stmt
            .query_map([], |row| {
                Ok(CostEvent {
                    timestamp_ms: row.get(0)?,
                    provider_id: row.get(1)?,
                    model: row.get(2)?,
                    prompt_tokens: row.get(3)?,
                    completion_tokens: row.get(4)?,
                    cost_usd: row.get(5)?,
                    latency_ms: row.get(6)?,
                    cached: row.get::<_, i32>(7)? != 0,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to query cost events")?;

        Ok(events)
    }

    async fn all_metrics(&self) -> Result<Vec<MetricEntry>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT timestamp_ms, metric_name, value, labels FROM metrics ORDER BY timestamp_ms ASC",
        )?;

        let metrics = stmt
            .query_map([], |row| {
                let labels_json: String = row.get(3)?;
                let labels: HashMap<String, String> =
                    serde_json::from_str(&labels_json).unwrap_or_default();
                Ok(MetricEntry {
                    timestamp_ms: row.get(0)?,
                    metric_name: row.get(1)?,
                    value: row.get(2)?,
                    labels,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to query metrics")?;

        Ok(metrics)
    }

    async fn ping(&self) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute_batch("SELECT 1")
            .context("SQLite ping failed")?;
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

    #[tokio::test]
    async fn test_sqlite_open_memory() {
        let store = SqliteEventStore::open_memory();
        assert!(store.is_ok(), "Should open in-memory database");
    }

    #[tokio::test]
    async fn test_sqlite_ping() {
        let store = SqliteEventStore::open_memory().expect("open");
        assert!(store.ping().await.is_ok());
    }

    #[tokio::test]
    async fn test_sqlite_record_and_retrieve() {
        let store = SqliteEventStore::open_memory().expect("open");
        let event = make_cost_event(0.05, 1000);

        store.record_cost(&event).await.expect("record");

        let events = store.all_cost_events().await.expect("get events");
        assert_eq!(events.len(), 1);
        assert!((events[0].cost_usd - 0.05).abs() < f64::EPSILON);
        assert_eq!(events[0].provider_id, "openai");
        assert_eq!(events[0].model, "gpt-4o");
    }

    #[tokio::test]
    async fn test_sqlite_multiple_records() {
        let store = SqliteEventStore::open_memory().expect("open");

        for i in 0..10 {
            let event = make_cost_event(0.01 * (i + 1) as f64, 1000 + i * 100);
            store.record_cost(&event).await.expect("record");
        }

        let events = store.all_cost_events().await.expect("get events");
        assert_eq!(events.len(), 10);
    }

    #[tokio::test]
    async fn test_sqlite_daily_costs() {
        let store = SqliteEventStore::open_memory().expect("open");

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // Insert events for today
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

        let total: f64 = daily.iter().sum();
        assert!((total - 0.30).abs() < 0.001, "Total should be 0.30, got {}", total);
    }

    #[tokio::test]
    async fn test_sqlite_daily_costs_empty() {
        let store = SqliteEventStore::open_memory().expect("open");
        let daily = store.daily_costs(30).await.expect("daily costs");
        assert!(daily.is_empty());
    }

    #[tokio::test]
    async fn test_sqlite_metrics() {
        let store = SqliteEventStore::open_memory().expect("open");

        // Record a metric by inserting directly (since record_metric is on InMemoryEventStore)
        let conn = store.conn.lock().await;
        conn.execute(
            "INSERT INTO metrics (timestamp_ms, metric_name, value, labels) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![1000, "cache_hit_rate", 0.85, "{}"],
        )
        .expect("insert metric");
        drop(conn);

        let metrics = store.all_metrics().await.expect("get metrics");
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].metric_name, "cache_hit_rate");
        assert!((metrics[0].value - 0.85).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_sqlite_cached_flag() {
        let store = SqliteEventStore::open_memory().expect("open");

        let mut event = make_cost_event(0.01, 1000);
        event.cached = true;
        store.record_cost(&event).await.expect("record");

        let events = store.all_cost_events().await.expect("get events");
        assert!(events[0].cached, "Cached flag should be preserved");
    }
}
