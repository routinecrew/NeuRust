//! # Cache Key Computation
//!
//! Computes a 64-bit hash for cache lookup using xxh64.
//! The hash includes: system message, last user message, model name,
//! turn count (messages.len()), and temperature (v3 design).

use std::hash::Hasher;

use xxhash_rust::xxh64::Xxh64;

use crate::contracts::UnifiedRequest;

/// Compute a deterministic cache key for a request.
///
/// Components hashed (per v3 design):
/// - System message text (all system messages joined)
/// - Last user message
/// - Model name
/// - Message count (turn count) to differentiate multi-turn conversations
/// - Temperature (if set) since different temperatures yield different outputs
pub fn compute_cache_key(request: &UnifiedRequest) -> u64 {
    let mut hasher = Xxh64::new(0);

    // System message
    let system_text = request.system_message_text();
    hasher.write(system_text.as_bytes());

    // Last user message
    let last_user = request.last_user_message();
    hasher.write(last_user.as_bytes());

    // Model name
    hasher.write(request.model.model_name.as_bytes());

    // Turn count (v3: prevent cross-turn collisions)
    hasher.write(&request.messages.len().to_le_bytes());

    // Temperature (v3: different temp = different response)
    // Use to_bits() for deterministic f64 hashing
    if let Some(temp) = request.temperature {
        hasher.write(&temp.to_bits().to_le_bytes());
    }

    // max_tokens: different limits produce different outputs
    if let Some(max_tokens) = request.max_tokens {
        hasher.write(&max_tokens.to_le_bytes());
    }

    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::*;
    use std::collections::HashMap;

    fn make_request_with(
        model: &str,
        messages: Vec<Message>,
        temperature: Option<f64>,
    ) -> UnifiedRequest {
        UnifiedRequest {
            model: ModelSpec {
                model_name: model.to_string(),
                provider_id: None,
            },
            messages,
            temperature,
            max_tokens: None,
            stream: false,
            context: RequestContext::default(),
            extra_params: HashMap::new(),
        }
    }

    #[test]
    fn test_same_request_same_hash() {
        let msgs = vec![Message {
            role: Role::User,
            content: "Hello".to_string(),
        }];
        let r1 = make_request_with("gpt-4o", msgs.clone(), Some(0.7));
        let r2 = make_request_with("gpt-4o", msgs, Some(0.7));

        assert_eq!(compute_cache_key(&r1), compute_cache_key(&r2));
    }

    #[test]
    fn test_different_model_different_hash() {
        let msgs = vec![Message {
            role: Role::User,
            content: "Hello".to_string(),
        }];
        let r1 = make_request_with("gpt-4o", msgs.clone(), Some(0.7));
        let r2 = make_request_with("gpt-4o-mini", msgs, Some(0.7));

        assert_ne!(compute_cache_key(&r1), compute_cache_key(&r2));
    }

    #[test]
    fn test_different_turn_count_different_hash() {
        let msgs_1turn = vec![Message {
            role: Role::User,
            content: "Hello".to_string(),
        }];
        let msgs_3turn = vec![
            Message {
                role: Role::User,
                content: "Hi".to_string(),
            },
            Message {
                role: Role::Assistant,
                content: "Hey".to_string(),
            },
            Message {
                role: Role::User,
                content: "Hello".to_string(),
            },
        ];

        let r1 = make_request_with("gpt-4o", msgs_1turn, Some(0.7));
        let r2 = make_request_with("gpt-4o", msgs_3turn, Some(0.7));

        assert_ne!(compute_cache_key(&r1), compute_cache_key(&r2));
    }

    #[test]
    fn test_different_temperature_different_hash() {
        let msgs = vec![Message {
            role: Role::User,
            content: "Hello".to_string(),
        }];
        let r1 = make_request_with("gpt-4o", msgs.clone(), Some(0.0));
        let r2 = make_request_with("gpt-4o", msgs, Some(1.0));

        assert_ne!(compute_cache_key(&r1), compute_cache_key(&r2));
    }

    #[test]
    fn test_different_max_tokens_different_hash() {
        let msgs = vec![Message {
            role: Role::User,
            content: "Hello".to_string(),
        }];
        let mut r1 = make_request_with("gpt-4o", msgs.clone(), Some(0.7));
        r1.max_tokens = Some(100);
        let mut r2 = make_request_with("gpt-4o", msgs, Some(0.7));
        r2.max_tokens = Some(500);

        assert_ne!(compute_cache_key(&r1), compute_cache_key(&r2));
    }

    #[test]
    fn test_no_temperature_vs_some_temperature() {
        let msgs = vec![Message {
            role: Role::User,
            content: "Hello".to_string(),
        }];
        let r1 = make_request_with("gpt-4o", msgs.clone(), None);
        let r2 = make_request_with("gpt-4o", msgs, Some(0.7));

        assert_ne!(compute_cache_key(&r1), compute_cache_key(&r2));
    }
}
