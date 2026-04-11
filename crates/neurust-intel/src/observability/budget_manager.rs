//! # Budget Manager
//!
//! Tracks spending against monthly budget limits.
//! Emits warnings at configurable thresholds and can hard-block requests at 100%.
//! Implements `PipelineLayer` for pipeline integration as a pre-check.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{Datelike, Utc};

use crate::contracts::{EventStore, PipelineLayer, UnifiedRequest, UnifiedResponse};

/// Budget status after checking current spend against the monthly limit.
#[derive(Clone, Debug, PartialEq)]
pub enum BudgetStatus {
    /// Spending is within normal range.
    Ok {
        /// Current spend as a ratio of the monthly budget (0.0-1.0).
        ratio: f64,
    },
    /// Spending has crossed the warning threshold.
    Warning {
        /// Current spend as a ratio of the monthly budget.
        ratio: f64,
        /// Remaining budget in USD.
        remaining: f64,
    },
    /// Spending has exceeded the monthly budget.
    Exceeded {
        /// Current spend as a ratio of the monthly budget (> 1.0).
        ratio: f64,
        /// Amount over the budget in USD.
        overage: f64,
    },
}

impl std::fmt::Display for BudgetStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ok { ratio } => write!(f, "ok ({:.1}%)", ratio * 100.0),
            Self::Warning { ratio, remaining } => {
                write!(f, "warning ({:.1}%, ${:.4} remaining)", ratio * 100.0, remaining)
            }
            Self::Exceeded { ratio, overage } => {
                write!(f, "exceeded ({:.1}%, ${:.4} over)", ratio * 100.0, overage)
            }
        }
    }
}

/// Configuration for the budget manager.
#[derive(Clone, Debug)]
pub struct BudgetManagerConfig {
    /// Monthly budget limit in USD.
    pub monthly_budget_usd: f64,
    /// Ratio at which to emit warnings (default: 0.8 = 80%).
    pub warn_threshold: f64,
    /// Whether to hard-block requests when budget is exceeded (default: false).
    pub hard_limit: bool,
    /// Day of month to reset the budget period (default: 1).
    pub reset_day: u8,
}

impl Default for BudgetManagerConfig {
    fn default() -> Self {
        Self {
            monthly_budget_usd: 100.0,
            warn_threshold: 0.8,
            hard_limit: false,
            reset_day: 1,
        }
    }
}

/// Tracks spending against a monthly budget.
///
/// Uses an atomic counter for lock-free spend tracking while supporting
/// full recalculation from the EventStore for accuracy.
pub struct BudgetManager {
    /// Budget configuration.
    config: BudgetManagerConfig,
    /// Event store for querying historical cost data.
    store: Arc<dyn EventStore>,
    /// Current period spend in micro-USD (1 USD = 1_000_000 micro-USD)
    /// to avoid floating-point atomics.
    current_spend_micro: AtomicU64,
}

impl BudgetManager {
    /// Create a new budget manager with the given config and event store.
    pub fn new(config: BudgetManagerConfig, store: Arc<dyn EventStore>) -> Self {
        Self {
            config,
            store,
            current_spend_micro: AtomicU64::new(0),
        }
    }

    /// Record a cost against the current billing period.
    pub fn record_cost(&self, cost_usd: f64) {
        let micro = (cost_usd * 1_000_000.0) as u64;
        let prev = self.current_spend_micro.fetch_add(micro, Ordering::Relaxed);
        let new_spend = (prev + micro) as f64 / 1_000_000.0;
        let ratio = new_spend / self.config.monthly_budget_usd;

        if ratio >= 1.0 {
            tracing::error!(
                spend = new_spend,
                budget = self.config.monthly_budget_usd,
                ratio = ratio,
                "budget exceeded"
            );
        } else if ratio >= self.config.warn_threshold {
            tracing::warn!(
                spend = new_spend,
                budget = self.config.monthly_budget_usd,
                ratio = ratio,
                "budget warning threshold reached"
            );
        }
    }

    /// Current spend as a ratio of the monthly budget (0.0 to unbounded).
    pub fn usage_ratio(&self) -> f64 {
        let spend = self.current_spend_micro.load(Ordering::Relaxed) as f64 / 1_000_000.0;
        spend / self.config.monthly_budget_usd
    }

    /// Remaining budget in USD. Returns 0.0 if over budget.
    pub fn remaining_budget(&self) -> f64 {
        let spend = self.current_spend_micro.load(Ordering::Relaxed) as f64 / 1_000_000.0;
        (self.config.monthly_budget_usd - spend).max(0.0)
    }

    /// Check the current budget status.
    pub fn check_budget(&self) -> BudgetStatus {
        let spend = self.current_spend_micro.load(Ordering::Relaxed) as f64 / 1_000_000.0;
        let ratio = spend / self.config.monthly_budget_usd;

        if ratio >= 1.0 {
            BudgetStatus::Exceeded {
                ratio,
                overage: spend - self.config.monthly_budget_usd,
            }
        } else if ratio >= self.config.warn_threshold {
            BudgetStatus::Warning {
                ratio,
                remaining: self.config.monthly_budget_usd - spend,
            }
        } else {
            BudgetStatus::Ok { ratio }
        }
    }

    /// Returns true if hard_limit is enabled and the budget is exceeded.
    pub fn should_block(&self) -> bool {
        self.config.hard_limit && self.usage_ratio() >= 1.0
    }

    /// Recalculate current spend from the event store for the current billing period.
    ///
    /// Queries all cost events and sums those within the current billing period
    /// based on `reset_day`. Replaces the atomic counter with the recalculated value.
    pub async fn refresh_from_store(&self) -> Result<()> {
        let events = self.store.all_cost_events().await?;
        let period_start_ms = billing_period_start_ms(self.config.reset_day);

        let total: f64 = events
            .iter()
            .filter(|e| e.timestamp_ms >= period_start_ms)
            .map(|e| e.cost_usd)
            .sum();

        let micro = (total * 1_000_000.0) as u64;
        self.current_spend_micro.store(micro, Ordering::Relaxed);

        tracing::debug!(
            total_spend = total,
            budget = self.config.monthly_budget_usd,
            "budget refreshed from store"
        );

        Ok(())
    }

    /// Access the current configuration.
    pub fn config(&self) -> &BudgetManagerConfig {
        &self.config
    }
}

#[async_trait]
impl PipelineLayer for BudgetManager {
    fn name(&self) -> &str {
        "budget-manager"
    }

    /// Pre-check: if hard_limit is on and budget is exceeded, reject the request.
    /// Also updates `budget_remaining_ratio` on the request context.
    async fn on_request(&self, request: &mut UnifiedRequest) -> Result<()> {
        request.context.budget_remaining_ratio = 1.0 - self.usage_ratio();

        if self.should_block() {
            return Err(anyhow::anyhow!(
                "budget exceeded: monthly limit of ${:.2} reached",
                self.config.monthly_budget_usd
            ));
        }

        Ok(())
    }

    /// Post-response: record the cost from the response's token usage.
    async fn on_response(
        &self,
        _request: &UnifiedRequest,
        response: &mut UnifiedResponse,
    ) -> Result<()> {
        // Estimate cost from token usage using a simple heuristic.
        // The CostTracker does the precise recording; here we just
        // update the budget counter with a rough estimate.
        let cost = estimate_response_cost(
            &response.model,
            response.usage.prompt_tokens,
            response.usage.completion_tokens,
        );
        self.record_cost(cost);
        Ok(())
    }
}

/// Estimate cost in USD based on model and token counts.
///
/// Mirrors the logic in cost_tracker for budget tracking purposes.
fn estimate_response_cost(model: &str, prompt_tokens: u32, completion_tokens: u32) -> f64 {
    let (prompt_price_per_1k, completion_price_per_1k) = match model {
        m if m.contains("gpt-4o-mini") => (0.00015, 0.0006),
        m if m.contains("gpt-4o") => (0.005, 0.015),
        m if m.contains("gpt-4") => (0.03, 0.06),
        m if m.contains("gpt-3.5") => (0.0005, 0.0015),
        m if m.contains("claude-3-opus") => (0.015, 0.075),
        m if m.contains("claude-sonnet-4") | m.contains("claude-3.5-sonnet") => (0.003, 0.015),
        m if m.contains("claude-haiku") => (0.00025, 0.00125),
        _ => (0.001, 0.002),
    };

    let prompt_cost = prompt_tokens as f64 / 1000.0 * prompt_price_per_1k;
    let completion_cost = completion_tokens as f64 / 1000.0 * completion_price_per_1k;

    prompt_cost + completion_cost
}

/// Compute the start of the current billing period as a Unix timestamp in milliseconds.
fn billing_period_start_ms(reset_day: u8) -> u64 {
    let now = Utc::now();
    let year = now.year();
    let month = now.month();
    let day = now.day();

    let (period_year, period_month) = if day >= reset_day as u32 {
        (year, month)
    } else if month == 1 {
        (year - 1, 12)
    } else {
        (year, month - 1)
    };

    let start = chrono::NaiveDate::from_ymd_opt(period_year, period_month, reset_day as u32)
        .unwrap_or_else(|| {
            // If reset_day exceeds days in month, use last day of month.
            let last_day = last_day_of_month(period_year, period_month);
            chrono::NaiveDate::from_ymd_opt(period_year, period_month, last_day)
                .expect("valid date")
        });

    let start_dt = start
        .and_hms_opt(0, 0, 0)
        .expect("valid time")
        .and_utc();

    start_dt.timestamp_millis() as u64
}

/// Return the last day of the given month.
fn last_day_of_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::*;
    use crate::mock::MockEventStore;
    use std::collections::HashMap;

    fn make_config(budget: f64, warn: f64, hard_limit: bool) -> BudgetManagerConfig {
        BudgetManagerConfig {
            monthly_budget_usd: budget,
            warn_threshold: warn,
            hard_limit,
            reset_day: 1,
        }
    }

    fn make_request() -> UnifiedRequest {
        UnifiedRequest {
            model: ModelSpec {
                model_name: "gpt-4o".to_string(),
                provider_id: None,
            },
            messages: vec![Message {
                role: Role::User,
                content: "Hello".to_string(),
            }],
            temperature: Some(0.7),
            max_tokens: Some(100),
            stream: false,
            context: RequestContext::default(),
            extra_params: HashMap::new(),
        }
    }

    fn make_response() -> UnifiedResponse {
        UnifiedResponse {
            content: "Hi there".to_string(),
            model: "gpt-4o".to_string(),
            usage: TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
            },
            provider_id: "openai".to_string(),
            latency_ms: 150,
            upstream_id: None,
        }
    }

    #[test]
    fn test_default_config() {
        let config = BudgetManagerConfig::default();
        assert!((config.monthly_budget_usd - 100.0).abs() < f64::EPSILON);
        assert!((config.warn_threshold - 0.8).abs() < f64::EPSILON);
        assert!(!config.hard_limit);
        assert_eq!(config.reset_day, 1);
    }

    #[test]
    fn test_record_cost_and_ratio() {
        let store = Arc::new(MockEventStore::new());
        let config = make_config(100.0, 0.8, false);
        let manager = BudgetManager::new(config, store);

        manager.record_cost(25.0);
        assert!((manager.usage_ratio() - 0.25).abs() < 0.001);
        assert!((manager.remaining_budget() - 75.0).abs() < 0.01);
    }

    #[test]
    fn test_check_budget_ok() {
        let store = Arc::new(MockEventStore::new());
        let config = make_config(100.0, 0.8, false);
        let manager = BudgetManager::new(config, store);

        manager.record_cost(50.0);
        match manager.check_budget() {
            BudgetStatus::Ok { ratio } => {
                assert!((ratio - 0.5).abs() < 0.001);
            }
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    #[test]
    fn test_check_budget_warning() {
        let store = Arc::new(MockEventStore::new());
        let config = make_config(100.0, 0.8, false);
        let manager = BudgetManager::new(config, store);

        manager.record_cost(85.0);
        match manager.check_budget() {
            BudgetStatus::Warning { ratio, remaining } => {
                assert!((ratio - 0.85).abs() < 0.001);
                assert!((remaining - 15.0).abs() < 0.01);
            }
            other => panic!("expected Warning, got {:?}", other),
        }
    }

    #[test]
    fn test_check_budget_exceeded() {
        let store = Arc::new(MockEventStore::new());
        let config = make_config(100.0, 0.8, false);
        let manager = BudgetManager::new(config, store);

        manager.record_cost(120.0);
        match manager.check_budget() {
            BudgetStatus::Exceeded { ratio, overage } => {
                assert!((ratio - 1.2).abs() < 0.001);
                assert!((overage - 20.0).abs() < 0.01);
            }
            other => panic!("expected Exceeded, got {:?}", other),
        }
    }

    #[test]
    fn test_should_block_when_hard_limit_off() {
        let store = Arc::new(MockEventStore::new());
        let config = make_config(100.0, 0.8, false);
        let manager = BudgetManager::new(config, store);

        manager.record_cost(150.0);
        assert!(!manager.should_block());
    }

    #[test]
    fn test_should_block_when_hard_limit_on() {
        let store = Arc::new(MockEventStore::new());
        let config = make_config(100.0, 0.8, true);
        let manager = BudgetManager::new(config, store);

        manager.record_cost(150.0);
        assert!(manager.should_block());
    }

    #[test]
    fn test_should_not_block_under_budget_with_hard_limit() {
        let store = Arc::new(MockEventStore::new());
        let config = make_config(100.0, 0.8, true);
        let manager = BudgetManager::new(config, store);

        manager.record_cost(50.0);
        assert!(!manager.should_block());
    }

    #[test]
    fn test_remaining_budget_zero_when_exceeded() {
        let store = Arc::new(MockEventStore::new());
        let config = make_config(100.0, 0.8, false);
        let manager = BudgetManager::new(config, store);

        manager.record_cost(150.0);
        assert!((manager.remaining_budget()).abs() < f64::EPSILON);
    }

    #[test]
    fn test_multiple_cost_records_accumulate() {
        let store = Arc::new(MockEventStore::new());
        let config = make_config(100.0, 0.8, false);
        let manager = BudgetManager::new(config, store);

        manager.record_cost(10.0);
        manager.record_cost(20.0);
        manager.record_cost(30.0);

        assert!((manager.usage_ratio() - 0.6).abs() < 0.001);
        assert!((manager.remaining_budget() - 40.0).abs() < 0.01);
    }

    #[test]
    fn test_budget_status_display() {
        let ok = BudgetStatus::Ok { ratio: 0.5 };
        assert!(ok.to_string().contains("50.0%"));

        let warn = BudgetStatus::Warning {
            ratio: 0.85,
            remaining: 15.0,
        };
        assert!(warn.to_string().contains("warning"));

        let exceeded = BudgetStatus::Exceeded {
            ratio: 1.2,
            overage: 20.0,
        };
        assert!(exceeded.to_string().contains("exceeded"));
    }

    #[tokio::test]
    async fn test_refresh_from_store() {
        let store = Arc::new(MockEventStore::new());

        // Record some events first
        let event = CostEvent {
            timestamp_ms: Utc::now().timestamp_millis() as u64,
            provider_id: "openai".to_string(),
            model: "gpt-4o".to_string(),
            prompt_tokens: 100,
            completion_tokens: 50,
            cost_usd: 25.0,
            latency_ms: 100,
            cached: false,
        };
        store.record_cost(&event).await.expect("should record");

        let config = make_config(100.0, 0.8, false);
        let manager = BudgetManager::new(config, store);

        manager.refresh_from_store().await.expect("should refresh");

        // The mock store returns all events; our single event has cost_usd=25.0
        // and its timestamp is current, so it falls within the billing period.
        assert!((manager.usage_ratio() - 0.25).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_pipeline_on_request_blocks_when_exceeded() {
        let store = Arc::new(MockEventStore::new());
        let config = make_config(100.0, 0.8, true);
        let manager = BudgetManager::new(config, store);

        manager.record_cost(150.0);

        let mut req = make_request();
        let result = manager.on_request(&mut req).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("budget exceeded"));
    }

    #[tokio::test]
    async fn test_pipeline_on_request_allows_when_under_budget() {
        let store = Arc::new(MockEventStore::new());
        let config = make_config(100.0, 0.8, true);
        let manager = BudgetManager::new(config, store);

        manager.record_cost(50.0);

        let mut req = make_request();
        let result = manager.on_request(&mut req).await;
        assert!(result.is_ok());
        assert!((req.context.budget_remaining_ratio - 0.5).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_pipeline_on_response_records_cost() {
        let store = Arc::new(MockEventStore::new());
        let config = make_config(100.0, 0.8, false);
        let manager = BudgetManager::new(config, store);

        let req = make_request();
        let mut resp = make_response();

        manager
            .on_response(&req, &mut resp)
            .await
            .expect("should succeed");

        // After recording, usage ratio should be > 0
        assert!(manager.usage_ratio() > 0.0);
    }

    #[test]
    fn test_estimate_response_cost() {
        let cost = estimate_response_cost("gpt-4o", 1000, 1000);
        assert!((cost - 0.02).abs() < 0.001);
    }

    #[test]
    fn test_billing_period_start_ms() {
        let ms = billing_period_start_ms(1);
        assert!(ms > 0);

        // The billing period start should be in the past
        let now_ms = Utc::now().timestamp_millis() as u64;
        assert!(ms <= now_ms);
    }

    #[test]
    fn test_last_day_of_month() {
        assert_eq!(last_day_of_month(2024, 2), 29); // leap year
        assert_eq!(last_day_of_month(2025, 2), 28);
        assert_eq!(last_day_of_month(2025, 1), 31);
        assert_eq!(last_day_of_month(2025, 4), 30);
    }
}
