//! # Sliding Window UCB (v3)
//!
//! Upper Confidence Bound algorithm with a sliding window over recent
//! observations. Unlike standard UCB1 which weights all history equally,
//! SW-UCB only uses the most recent `window_size` observations per arm,
//! allowing faster adaptation to provider performance changes.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

/// A single reward observation for a model arm.
struct Observation {
    /// Reward value: 0.0 (failure/slow) to 1.0 (success/fast).
    reward: f64,
    /// When this observation was recorded.
    _timestamp: Instant,
}

/// Maximum number of distinct arms to prevent unbounded HashMap growth.
const MAX_ARMS: usize = 1000;

/// Sliding Window Upper Confidence Bound bandit.
///
/// Tracks per-model performance observations within a fixed-size sliding window.
/// Computes UCB scores combining exploitation (mean reward) and exploration
/// (confidence bound based on observation count).
pub struct SlidingWindowUCB {
    /// Maximum observations per arm.
    window_size: usize,
    /// Per-model observation history.
    arms: HashMap<String, VecDeque<Observation>>,
    /// Total observations across all arms (within their windows).
    total_observations: u64,
}

impl SlidingWindowUCB {
    /// Create a new SW-UCB bandit with the given window size.
    pub fn new(window_size: usize) -> Self {
        Self {
            window_size,
            arms: HashMap::new(),
            total_observations: 0,
        }
    }

    /// Get the UCB score for a given model.
    ///
    /// Returns 0.5 (neutral) if no observations exist for the model.
    /// Score = mean_reward + sqrt(2 * ln(total_n) / n)
    pub fn get_score(&self, model_id: &str) -> f64 {
        let arm = match self.arms.get(model_id) {
            Some(a) if !a.is_empty() => a,
            _ => return 0.5, // no data, neutral score
        };

        let n = arm.len() as f64;
        let total_n: f64 = self
            .arms
            .values()
            .map(|a| a.len() as f64)
            .sum();

        if total_n <= 0.0 || n <= 0.0 {
            return 0.5;
        }

        let mean_reward = arm.iter().map(|o| o.reward).sum::<f64>() / n;

        // UCB exploration bonus
        let ln_total = total_n.ln();
        let exploration = if ln_total > 0.0 {
            (2.0 * ln_total / n).sqrt()
        } else {
            0.0
        };

        mean_reward + exploration
    }

    /// Record a reward observation for a model.
    ///
    /// If the window is full, the oldest observation is evicted.
    /// If max arms limit is reached, the least-used arm is removed to prevent unbounded growth.
    pub fn update(&mut self, model_id: &str, reward: f64) {
        // Prevent unbounded growth: evict least-used arm if at capacity
        if !self.arms.contains_key(model_id) && self.arms.len() >= MAX_ARMS {
            if let Some(least_used) = self
                .arms
                .iter()
                .min_by_key(|(_, obs)| obs.len())
                .map(|(k, _)| k.clone())
            {
                tracing::warn!(
                    evicted_arm = %least_used,
                    max_arms = MAX_ARMS,
                    "UCB arm limit reached, evicting least-used arm"
                );
                self.arms.remove(&least_used);
            }
        }

        let arm = self.arms.entry(model_id.to_string()).or_default();
        arm.push_back(Observation {
            reward,
            _timestamp: Instant::now(),
        });

        self.total_observations = self.total_observations.saturating_add(1);

        // Evict oldest if over window size
        while arm.len() > self.window_size {
            arm.pop_front();
        }
    }

    /// Total number of observations recorded (lifetime, not windowed).
    pub fn total_requests(&self) -> u64 {
        self.total_observations
    }

    /// Number of observations currently in the window for a model.
    pub fn arm_count(&self, model_id: &str) -> usize {
        self.arms
            .get(model_id)
            .map(|a| a.len())
            .unwrap_or(0)
    }

    /// All known model IDs.
    pub fn model_ids(&self) -> Vec<String> {
        self.arms.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_data_returns_neutral() {
        let bandit = SlidingWindowUCB::new(100);
        assert!((bandit.get_score("gpt-4o") - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_update_and_score() {
        let mut bandit = SlidingWindowUCB::new(100);

        // Model A: consistently good
        for _ in 0..10 {
            bandit.update("model-a", 0.9);
        }
        // Model B: consistently bad
        for _ in 0..10 {
            bandit.update("model-b", 0.2);
        }

        let score_a = bandit.get_score("model-a");
        let score_b = bandit.get_score("model-b");

        // Model A should score higher than Model B
        assert!(score_a > score_b, "A={} should be > B={}", score_a, score_b);
    }

    #[test]
    fn test_window_eviction() {
        let mut bandit = SlidingWindowUCB::new(5);

        for _ in 0..10 {
            bandit.update("model-a", 0.5);
        }

        assert_eq!(bandit.arm_count("model-a"), 5);
        assert_eq!(bandit.total_requests(), 10);
    }

    #[test]
    fn test_max_arms_limit() {
        let mut bandit = SlidingWindowUCB::new(10);
        // Insert MAX_ARMS + 10 arms
        for i in 0..super::MAX_ARMS + 10 {
            bandit.update(&format!("model-{}", i), 0.5);
        }
        // Should not exceed MAX_ARMS
        assert!(bandit.model_ids().len() <= super::MAX_ARMS, "Arms should be bounded to MAX_ARMS");
    }

    #[test]
    fn test_exploration_bonus_for_undersampled() {
        let mut bandit = SlidingWindowUCB::new(100);

        // Model A: many observations, decent reward
        for _ in 0..100 {
            bandit.update("model-a", 0.6);
        }
        // Model B: few observations, same reward
        for _ in 0..2 {
            bandit.update("model-b", 0.6);
        }

        let score_a = bandit.get_score("model-a");
        let score_b = bandit.get_score("model-b");

        // Model B should get a higher exploration bonus
        assert!(
            score_b > score_a,
            "Undersampled B={} should get exploration bonus > A={}",
            score_b,
            score_a
        );
    }
}
