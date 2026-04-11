//! # Complexity Gate
//!
//! Classifies request complexity to route to appropriate model tier.
//! Simple requests go to cheaper/faster models, complex ones to capable models.

use crate::contracts::{ModelSpec, UnifiedRequest, Role};

/// Complexity classification for a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComplexityLevel {
    /// Short, single-turn, no tools — suitable for nano/mini models.
    Simple,
    /// Moderate conversation length or system prompt — mid-tier models.
    Medium,
    /// Long context, multi-turn, tool use — requires capable models.
    Complex,
}

impl std::fmt::Display for ComplexityLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Simple => write!(f, "simple"),
            Self::Medium => write!(f, "medium"),
            Self::Complex => write!(f, "complex"),
        }
    }
}

/// Configuration thresholds for complexity classification.
#[derive(Clone, Debug)]
pub struct ComplexityGateConfig {
    /// Maximum estimated token count for a request to be classified as Simple.
    pub simple_max_tokens: usize,
    /// Minimum estimated token count for a request to be classified as Complex.
    pub complex_min_tokens: usize,
    /// Weight multiplier applied to system prompt tokens when estimating complexity.
    /// System prompts typically indicate structured/constrained tasks.
    pub system_prompt_weight: f64,
}

impl Default for ComplexityGateConfig {
    fn default() -> Self {
        Self {
            simple_max_tokens: 200,
            complex_min_tokens: 2000,
            system_prompt_weight: 1.5,
        }
    }
}

/// Classifies request complexity and filters model candidates by tier.
///
/// Uses heuristics based on token count, message count, system prompt presence,
/// and tool/function usage to determine whether a request is simple, medium,
/// or complex. This drives model selection toward cost-appropriate tiers.
#[derive(Clone, Debug)]
pub struct ComplexityGate {
    config: ComplexityGateConfig,
}

impl ComplexityGate {
    /// Create a new ComplexityGate with the given configuration.
    pub fn new(config: ComplexityGateConfig) -> Self {
        Self { config }
    }

    /// Classify the complexity of a request.
    ///
    /// Classification is based on:
    /// - Total message token count (estimated by char count / 4)
    /// - Number of messages in conversation
    /// - Presence of system prompt (weighted by `system_prompt_weight`)
    /// - Whether tools/functions are present in extra_params
    pub fn classify(&self, request: &UnifiedRequest) -> ComplexityLevel {
        let mut estimated_tokens: f64 = 0.0;

        for msg in &request.messages {
            let msg_tokens = estimate_tokens(&msg.content);
            if msg.role == Role::System {
                estimated_tokens += msg_tokens as f64 * self.config.system_prompt_weight;
            } else {
                estimated_tokens += msg_tokens as f64;
            }
        }

        let message_count = request.messages.len();
        let has_tools = request.extra_params.contains_key("tools")
            || request.extra_params.contains_key("functions")
            || request.extra_params.contains_key("tool_choice");

        // Tool usage is a strong signal for complex requests
        if has_tools {
            return ComplexityLevel::Complex;
        }

        // Multi-turn conversations with many messages trend toward complex
        if message_count > 10 {
            estimated_tokens *= 1.3;
        } else if message_count > 5 {
            estimated_tokens *= 1.1;
        }

        let total = estimated_tokens as usize;

        if total <= self.config.simple_max_tokens {
            ComplexityLevel::Simple
        } else if total >= self.config.complex_min_tokens {
            ComplexityLevel::Complex
        } else {
            ComplexityLevel::Medium
        }
    }

    /// Filter model candidates by complexity tier.
    ///
    /// Model tier is determined by naming convention:
    /// - Simple: models containing "nano", "mini", or "haiku"
    /// - Medium: models containing "mini", "haiku", "sonnet", or mid-tier names
    /// - Complex: all models are eligible (no filtering)
    ///
    /// Returns the original list if filtering would produce an empty result.
    pub fn filter_models(
        &self,
        level: ComplexityLevel,
        models: &[ModelSpec],
    ) -> Vec<ModelSpec> {
        if models.is_empty() {
            return vec![];
        }

        let filtered: Vec<ModelSpec> = match level {
            ComplexityLevel::Simple => models
                .iter()
                .filter(|m| {
                    let name = m.model_name.to_lowercase();
                    name.contains("nano")
                        || name.contains("mini")
                        || name.contains("haiku")
                })
                .cloned()
                .collect(),
            ComplexityLevel::Medium => models
                .iter()
                .filter(|m| {
                    let name = m.model_name.to_lowercase();
                    name.contains("mini")
                        || name.contains("haiku")
                        || name.contains("sonnet")
                        || name.contains("4o")
                        || name.contains("4.1")
                })
                .cloned()
                .collect(),
            ComplexityLevel::Complex => models.to_vec(),
        };

        // Fallback: never return empty if we had candidates
        if filtered.is_empty() {
            models.to_vec()
        } else {
            filtered
        }
    }
}

/// Estimate token count from text using the ~4 chars per token heuristic.
fn estimate_tokens(text: &str) -> usize {
    (text.len() / 4).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::*;
    use std::collections::HashMap;

    fn make_request(messages: Vec<Message>) -> UnifiedRequest {
        UnifiedRequest {
            model: ModelSpec {
                model_name: "gpt-4o".to_string(),
                provider_id: None,
            },
            messages,
            temperature: Some(0.7),
            max_tokens: Some(100),
            stream: false,
            context: RequestContext::default(),
            extra_params: HashMap::new(),
        }
    }

    fn make_request_with_tools(messages: Vec<Message>) -> UnifiedRequest {
        let mut extra = HashMap::new();
        extra.insert(
            "tools".to_string(),
            serde_json::json!([{"type": "function", "name": "search"}]),
        );
        UnifiedRequest {
            model: ModelSpec {
                model_name: "gpt-4o".to_string(),
                provider_id: None,
            },
            messages,
            temperature: Some(0.7),
            max_tokens: Some(100),
            stream: false,
            context: RequestContext::default(),
            extra_params: extra,
        }
    }

    #[test]
    fn test_simple_classification() {
        let gate = ComplexityGate::new(ComplexityGateConfig::default());
        let request = make_request(vec![Message {
            role: Role::User,
            content: "Hello".to_string(),
        }]);

        assert_eq!(gate.classify(&request), ComplexityLevel::Simple);
    }

    #[test]
    fn test_complex_with_tools() {
        let gate = ComplexityGate::new(ComplexityGateConfig::default());
        let request = make_request_with_tools(vec![Message {
            role: Role::User,
            content: "Search for X".to_string(),
        }]);

        assert_eq!(gate.classify(&request), ComplexityLevel::Complex);
    }

    #[test]
    fn test_complex_long_context() {
        let gate = ComplexityGate::new(ComplexityGateConfig::default());
        // Create a long message (>2000 tokens estimated = >8000 chars)
        let long_text = "a".repeat(10000);
        let request = make_request(vec![Message {
            role: Role::User,
            content: long_text,
        }]);

        assert_eq!(gate.classify(&request), ComplexityLevel::Complex);
    }

    #[test]
    fn test_medium_classification() {
        let gate = ComplexityGate::new(ComplexityGateConfig::default());
        // ~1000 chars = ~250 tokens, above simple_max_tokens(200) but below complex_min_tokens(2000)
        let text = "a".repeat(1200);
        let request = make_request(vec![Message {
            role: Role::User,
            content: text,
        }]);

        assert_eq!(gate.classify(&request), ComplexityLevel::Medium);
    }

    #[test]
    fn test_system_prompt_weight() {
        let gate = ComplexityGate::new(ComplexityGateConfig {
            simple_max_tokens: 100,
            complex_min_tokens: 500,
            system_prompt_weight: 3.0,
        });

        // 400 chars = 100 tokens normally, but with system_prompt_weight=3.0 -> 300 tokens
        let request = make_request(vec![Message {
            role: Role::System,
            content: "a".repeat(400),
        }]);

        assert_eq!(gate.classify(&request), ComplexityLevel::Medium);
    }

    #[test]
    fn test_multi_turn_boost() {
        let gate = ComplexityGate::new(ComplexityGateConfig {
            simple_max_tokens: 200,
            complex_min_tokens: 500,
            system_prompt_weight: 1.0,
        });

        // 12 messages with ~30 tokens each = ~360 tokens, boosted by 1.3 = ~468
        let messages: Vec<Message> = (0..12)
            .map(|i| Message {
                role: if i % 2 == 0 {
                    Role::User
                } else {
                    Role::Assistant
                },
                content: "a".repeat(120),
            })
            .collect();

        let request = make_request(messages);
        let level = gate.classify(&request);

        // Should be at least Medium due to multi-turn boost
        assert_ne!(level, ComplexityLevel::Simple);
    }

    #[test]
    fn test_filter_models_simple() {
        let gate = ComplexityGate::new(ComplexityGateConfig::default());
        let models = vec![
            ModelSpec {
                model_name: "gpt-4o".to_string(),
                provider_id: None,
            },
            ModelSpec {
                model_name: "gpt-4o-mini".to_string(),
                provider_id: None,
            },
            ModelSpec {
                model_name: "gpt-4.1-nano".to_string(),
                provider_id: None,
            },
            ModelSpec {
                model_name: "claude-opus-4-6".to_string(),
                provider_id: None,
            },
            ModelSpec {
                model_name: "claude-haiku-4-5-20251001".to_string(),
                provider_id: None,
            },
        ];

        let filtered = gate.filter_models(ComplexityLevel::Simple, &models);
        assert_eq!(filtered.len(), 3);
        assert!(filtered.iter().all(|m| {
            let n = m.model_name.to_lowercase();
            n.contains("mini") || n.contains("nano") || n.contains("haiku")
        }));
    }

    #[test]
    fn test_filter_models_complex_returns_all() {
        let gate = ComplexityGate::new(ComplexityGateConfig::default());
        let models = vec![
            ModelSpec {
                model_name: "gpt-4o".to_string(),
                provider_id: None,
            },
            ModelSpec {
                model_name: "gpt-4o-mini".to_string(),
                provider_id: None,
            },
        ];

        let filtered = gate.filter_models(ComplexityLevel::Complex, &models);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_models_empty_input() {
        let gate = ComplexityGate::new(ComplexityGateConfig::default());
        let filtered = gate.filter_models(ComplexityLevel::Simple, &[]);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_models_fallback_when_no_match() {
        let gate = ComplexityGate::new(ComplexityGateConfig::default());
        // No mini/nano/haiku models available
        let models = vec![ModelSpec {
            model_name: "claude-opus-4-6".to_string(),
            provider_id: None,
        }];

        let filtered = gate.filter_models(ComplexityLevel::Simple, &models);
        // Should fall back to all models instead of empty
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn test_complexity_level_display() {
        assert_eq!(ComplexityLevel::Simple.to_string(), "simple");
        assert_eq!(ComplexityLevel::Medium.to_string(), "medium");
        assert_eq!(ComplexityLevel::Complex.to_string(), "complex");
    }

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 1); // min 1
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("a".repeat(400).as_str()), 100);
    }
}
