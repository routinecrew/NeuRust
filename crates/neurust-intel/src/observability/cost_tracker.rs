//! # Cost Tracker
//!
//! Per-request cost tracking. Records cost events to the EventStore
//! on each completed response. Implements `PipelineLayer` for pipeline
//! integration.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use async_trait::async_trait;
use tracing;

use crate::contracts::{CostEvent, EventStore, PipelineLayer, UnifiedRequest, UnifiedResponse};

/// Per-request cost tracker.
///
/// Computes estimated cost based on token usage and records it
/// to the EventStore on each response.
pub struct CostTracker {
    /// Event store for persisting cost events.
    store: Arc<dyn EventStore>,
}

impl CostTracker {
    /// Create a new cost tracker with the given event store.
    pub fn new(store: Arc<dyn EventStore>) -> Self {
        Self { store }
    }

    /// Record a cost event for the given request/response pair.
    pub async fn record(&self, request: &UnifiedRequest, response: &UnifiedResponse) -> Result<()> {
        let cost = estimate_cost(
            &response.model,
            response.usage.prompt_tokens,
            response.usage.completion_tokens,
        );

        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let event = CostEvent {
            timestamp_ms,
            provider_id: response.provider_id.clone(),
            model: response.model.clone(),
            prompt_tokens: response.usage.prompt_tokens,
            completion_tokens: response.usage.completion_tokens,
            cost_usd: cost,
            latency_ms: response.latency_ms,
            cached: request
                .extra_params
                .get("cached")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        };

        self.store.record_cost(&event).await?;

        tracing::debug!(
            model = %response.model,
            cost_usd = cost,
            tokens = response.usage.total_tokens,
            "Cost event recorded"
        );

        Ok(())
    }
}

#[async_trait]
impl PipelineLayer for CostTracker {
    fn name(&self) -> &str {
        "cost-tracker"
    }

    async fn on_response(
        &self,
        request: &UnifiedRequest,
        response: &mut UnifiedResponse,
    ) -> Result<()> {
        self.record(request, response).await
    }
}

/// Estimate cost in USD based on model and token counts.
///
/// Uses approximate pricing. In production, this should be driven
/// by `data/model_prices.json`.
fn estimate_cost(model: &str, prompt_tokens: u32, completion_tokens: u32) -> f64 {
    let (prompt_price_per_1k, completion_price_per_1k) = match model {
        m if m.contains("gpt-4o-mini") => (0.00015, 0.0006),
        m if m.contains("gpt-4o") => (0.005, 0.015),
        m if m.contains("gpt-4") => (0.03, 0.06),
        m if m.contains("gpt-3.5") => (0.0005, 0.0015),
        m if m.contains("claude-3-opus") => (0.015, 0.075),
        m if m.contains("claude-sonnet-4") | m.contains("claude-3.5-sonnet") => (0.003, 0.015),
        m if m.contains("claude-haiku") => (0.00025, 0.00125),
        _ => (0.001, 0.002), // default fallback
    };

    let prompt_cost = prompt_tokens as f64 / 1000.0 * prompt_price_per_1k;
    let completion_cost = completion_tokens as f64 / 1000.0 * completion_price_per_1k;

    prompt_cost + completion_cost
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::*;
    use crate::mock::MockEventStore;
    use std::collections::HashMap;

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
    fn test_estimate_cost_gpt4o() {
        let cost = estimate_cost("gpt-4o", 1000, 1000);
        // 1K prompt * 0.005 + 1K completion * 0.015 = 0.02
        assert!((cost - 0.02).abs() < 0.001);
    }

    #[test]
    fn test_estimate_cost_gpt4o_mini() {
        let cost = estimate_cost("gpt-4o-mini", 1000, 1000);
        // 1K * 0.00015 + 1K * 0.0006 = 0.00075
        assert!((cost - 0.00075).abs() < 0.0001);
    }

    #[tokio::test]
    async fn test_cost_tracker_record() {
        let store = Arc::new(MockEventStore::new());
        let tracker = CostTracker::new(store.clone());

        let req = make_request();
        let resp = make_response();

        tracker.record(&req, &resp).await.expect("record should succeed");

        let events = store.all_cost_events().await.expect("should get events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].model, "gpt-4o");
        assert!(events[0].cost_usd > 0.0);
    }
}
