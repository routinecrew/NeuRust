//! # Cost Auctioneer (v3)
//!
//! Scores candidate models by weighted combination of cost, latency, and quality,
//! using context-based dynamic weights. Includes safe normalization for edge cases
//! (1 candidate, identical values) and epsilon-greedy exploration.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing;

use crate::contracts::{
    ContextWeights, CostRouterConfig, ModelSpec, PipelineLayer, RequestContext,
    UnifiedRequest, UnifiedResponse,
};

use super::bandit::SlidingWindowUCB;

/// Raw score data for a single candidate model.
#[derive(Clone, Debug)]
pub struct RawScore {
    /// Model identifier.
    pub model_id: String,
    /// Estimated cost (USD per 1K tokens).
    pub cost: f64,
    /// Estimated latency in milliseconds.
    pub latency_ms: f64,
    /// Quality score (0.0 ~ 1.0).
    pub quality: f64,
}

/// Cost-aware model auctioneer.
///
/// Scores models using weighted (cost, latency, quality) and selects the best
/// candidate. Uses epsilon-greedy exploration and integrates with SW-UCB bandit
/// for adaptive learning.
pub struct CostAuctioneer {
    /// Configuration for the router.
    config: CostRouterConfig,
    /// Bandit for exploration/exploitation.
    bandit: Arc<RwLock<SlidingWindowUCB>>,
    /// Known candidate models and their static attributes.
    candidates: Vec<RawScore>,
}

impl CostAuctioneer {
    /// Create a new CostAuctioneer.
    pub fn new(config: CostRouterConfig, candidates: Vec<RawScore>) -> Self {
        let window_size = config.sliding_window_size.unwrap_or(500);
        Self {
            config,
            bandit: Arc::new(RwLock::new(SlidingWindowUCB::new(window_size))),
            candidates,
        }
    }

    /// Select the best model for the given request.
    pub async fn select_best(&self, request: &UnifiedRequest) -> Result<ModelSpec> {
        if self.candidates.is_empty() {
            return Err(anyhow::anyhow!("No candidate models available"));
        }

        // Cold start: use priority ordering until enough data
        let bandit = self.bandit.read().await;
        let warmup = self.config.warmup_requests.unwrap_or(100);
        if bandit.total_requests() < warmup {
            drop(bandit);
            return self.select_first();
        }
        drop(bandit);

        // Collect raw scores
        let costs: Vec<f64> = self.candidates.iter().map(|s| s.cost).collect();
        let latencies: Vec<f64> = self.candidates.iter().map(|s| s.latency_ms).collect();

        // Context-based dynamic weights
        let weights = self.get_context_weights(&request.context);
        let budget_factor = if request.context.budget_remaining_ratio < 0.2 {
            1.5
        } else {
            1.0
        };

        let mut scored: Vec<(String, f64)> = self
            .candidates
            .iter()
            .map(|s| {
                let cost_norm = safe_normalize(s.cost, &costs);
                let lat_norm = safe_normalize(s.latency_ms, &latencies);
                let qual_norm = s.quality;

                // Lower combined score = better (cost and latency are costs, quality is inverted)
                let combined = cost_norm * weights.cost * budget_factor
                    + lat_norm * weights.latency
                    + (1.0 - qual_norm) * weights.quality;

                (s.model_id.clone(), combined)
            })
            .collect();

        // Epsilon-greedy exploration
        let epsilon = self.config.epsilon.unwrap_or(0.1);
        if rand_f64() < epsilon {
            return self.random_choice(&scored);
        }

        // Sort by combined score (ascending = better)
        scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        self.to_model_spec(&scored[0].0)
    }

    /// Record an observation for bandit learning.
    pub async fn record_observation(&self, model_id: &str, reward: f64) {
        let mut bandit = self.bandit.write().await;
        bandit.update(model_id, reward);
    }

    /// Get context-based weights (interactive vs batch).
    fn get_context_weights(&self, context: &RequestContext) -> ContextWeights {
        let request_type = context
            .request_type
            .as_deref()
            .unwrap_or("interactive");

        if let Some(ref weights_map) = self.config.context_weights {
            if let Some(w) = weights_map.get(request_type) {
                return w.clone();
            }
        }

        // Default weights for interactive
        ContextWeights {
            cost: 0.2,
            latency: 0.6,
            quality: 0.2,
        }
    }

    /// Select the first candidate (priority-based fallback).
    fn select_first(&self) -> Result<ModelSpec> {
        let first = self
            .candidates
            .first()
            .ok_or_else(|| anyhow::anyhow!("No candidates"))?;
        self.to_model_spec(&first.model_id)
    }

    /// Randomly select from the scored candidates.
    fn random_choice(&self, scored: &[(String, f64)]) -> Result<ModelSpec> {
        if scored.is_empty() {
            return Err(anyhow::anyhow!("No scored candidates"));
        }
        let idx = (rand_f64() * scored.len() as f64) as usize;
        let idx = idx.min(scored.len() - 1);
        self.to_model_spec(&scored[idx].0)
    }

    /// Convert a model ID to a ModelSpec.
    fn to_model_spec(&self, model_id: &str) -> Result<ModelSpec> {
        Ok(ModelSpec {
            model_name: model_id.to_string(),
            provider_id: None,
        })
    }
}

#[async_trait]
impl PipelineLayer for CostAuctioneer {
    fn name(&self) -> &str {
        "cost-auctioneer"
    }

    async fn on_request(&self, request: &mut UnifiedRequest) -> Result<()> {
        let selected = self.select_best(request).await?;
        tracing::debug!(
            model = %selected.model_name,
            "CostAuctioneer selected model"
        );
        request.model = selected;
        Ok(())
    }

    async fn on_response(
        &self,
        request: &UnifiedRequest,
        response: &mut UnifiedResponse,
    ) -> Result<()> {
        // Compute reward: lower latency = higher reward
        let max_latency = 5000.0_f64;
        let reward = 1.0 - (response.latency_ms as f64 / max_latency).min(1.0);
        self.record_observation(&request.model.model_name, reward)
            .await;
        Ok(())
    }
}

/// Safe normalization: handles 1-candidate and min==max edge cases.
///
/// - 1 candidate: returns 0.5 (neutral)
/// - min == max: returns 0.5 for all (tie)
/// - Normal: min-max normalization to 0.0..1.0
/// Safe normalization: handles 1-candidate, min==max, and NaN edge cases.
///
/// - NaN/Inf values are filtered out before computation
/// - 1 candidate: returns 0.5 (neutral)
/// - min == max: returns 0.5 for all (tie)
/// - Normal: min-max normalization to 0.0..1.0
pub fn safe_normalize(value: f64, all_values: &[f64]) -> f64 {
    if !value.is_finite() {
        return 0.5;
    }

    // Filter out NaN/Inf values
    let finite_values: Vec<f64> = all_values.iter().copied().filter(|v| v.is_finite()).collect();

    if finite_values.len() <= 1 {
        return 0.5;
    }

    let min = finite_values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = finite_values
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;

    if range < f64::EPSILON {
        return 0.5;
    }

    ((value - min) / range).clamp(0.0, 1.0)
}

/// Simple pseudo-random f64 in [0.0, 1.0).
/// Uses current system time nanos for lightweight randomness.
/// Not cryptographically secure — suitable for exploration only.
fn rand_f64() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // Use lower bits which have more entropy
    ((nanos ^ (nanos >> 17)) % 10000) as f64 / 10000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_normalize_single_value() {
        assert!((safe_normalize(5.0, &[5.0]) - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_safe_normalize_equal_values() {
        assert!((safe_normalize(3.0, &[3.0, 3.0, 3.0]) - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_safe_normalize_range() {
        let values = vec![10.0, 20.0, 30.0];

        let norm_min = safe_normalize(10.0, &values);
        let norm_mid = safe_normalize(20.0, &values);
        let norm_max = safe_normalize(30.0, &values);

        assert!((norm_min - 0.0).abs() < f64::EPSILON);
        assert!((norm_mid - 0.5).abs() < f64::EPSILON);
        assert!((norm_max - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_safe_normalize_empty() {
        assert!((safe_normalize(5.0, &[]) - 0.5).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_auctioneer_no_candidates() {
        let config = CostRouterConfig {
            enabled: true,
            warmup_requests: Some(0),
            epsilon: Some(0.0),
            normalize_mode: None,
            bandit_algorithm: None,
            sliding_window_size: Some(100),
            context_weights: None,
        };
        let auctioneer = CostAuctioneer::new(config, vec![]);

        let request = make_test_request();
        let result = auctioneer.select_best(&request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_auctioneer_cold_start() {
        let config = CostRouterConfig {
            enabled: true,
            warmup_requests: Some(100),
            epsilon: Some(0.0),
            normalize_mode: None,
            bandit_algorithm: None,
            sliding_window_size: Some(500),
            context_weights: None,
        };
        let candidates = vec![
            RawScore {
                model_id: "gpt-4o".to_string(),
                cost: 0.03,
                latency_ms: 200.0,
                quality: 0.9,
            },
            RawScore {
                model_id: "gpt-4o-mini".to_string(),
                cost: 0.01,
                latency_ms: 100.0,
                quality: 0.7,
            },
        ];
        let auctioneer = CostAuctioneer::new(config, candidates);
        let request = make_test_request();

        // During cold start, should return first candidate
        let result = auctioneer.select_best(&request).await;
        assert!(result.is_ok());
        let model = result.expect("should succeed");
        assert_eq!(model.model_name, "gpt-4o");
    }

    #[test]
    fn test_safe_normalize_with_nan() {
        let values = vec![f64::NAN, 1.0, 2.0];
        let result = safe_normalize(1.0, &values);
        assert!(result.is_finite(), "NaN in values should not propagate: got {}", result);
    }

    #[test]
    fn test_safe_normalize_nan_value() {
        let values = vec![1.0, 2.0, 3.0];
        let result = safe_normalize(f64::NAN, &values);
        assert!((result - 0.5).abs() < f64::EPSILON, "NaN input should return 0.5");
    }

    #[test]
    fn test_safe_normalize_infinity() {
        let values = vec![f64::INFINITY, 1.0, 2.0];
        let result = safe_normalize(1.0, &values);
        assert!(result.is_finite());
    }

    fn make_test_request() -> UnifiedRequest {
        use crate::contracts::*;
        use std::collections::HashMap;

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
}
