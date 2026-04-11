//! Integration tests for the NeuRust gateway HTTP layer.
//!
//! These tests spin up a real Axum server with mock providers
//! and validate request/response behavior end-to-end.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use neurust_gateway::contracts::*;
use neurust_gateway::mock::MockProvider;
use neurust_gateway::server::build_router;
use neurust_gateway::state::AppState;

/// Build a test AppState with mock providers and no auth.
fn test_state() -> AppState {
    let providers: Vec<Arc<dyn Provider>> = vec![
        Arc::new(MockProvider::openai()),
        Arc::new(MockProvider::anthropic()),
    ];
    let config = NeuRustConfig {
        server: ServerConfig {
            address: "127.0.0.1:0".to_string(),
            graceful_shutdown_sec: None,
            max_stream_buffer_tokens: None,
        },
        providers: vec![
            ProviderConfig {
                id: "mock-openai".to_string(),
                provider_type: "openai".to_string(),
                api_key_env: "FAKE".to_string(),
                base_url: None,
                models: vec!["gpt-4o".into(), "gpt-4o-mini".into()],
                priority: Some(1),
                max_connections: None,
            },
            ProviderConfig {
                id: "mock-anthropic".to_string(),
                provider_type: "anthropic".to_string(),
                api_key_env: "FAKE".to_string(),
                base_url: None,
                models: vec!["claude-sonnet-4-20250514".into()],
                priority: Some(2),
                max_connections: None,
            },
        ],
        intelligence: None,
        security: None,
        observability: None,
        auth: None,
    };

    let (event_tx, _) = new_event_bus();
    AppState::new(providers, config, event_tx)
}

fn app() -> axum::Router {
    build_router(test_state())
}

// ---- Health Endpoints ----

#[tokio::test]
async fn test_liveness_returns_200() {
    let app = app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_readiness_returns_200_with_healthy_providers() {
    let app = app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ready");
    assert!(json["providers"].as_array().unwrap().len() >= 2);
}

// ---- Models Endpoint ----

#[tokio::test]
async fn test_list_models() {
    let app = app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["object"], "list");
    assert!(!json["data"].as_array().unwrap().is_empty());
}

// ---- Chat Completions ----

#[tokio::test]
async fn test_chat_completion_non_streaming() {
    let app = app();
    let body = serde_json::json!({
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "Hello"}]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["object"], "chat.completion");
    assert!(!json["choices"].as_array().unwrap().is_empty());
    assert!(json["usage"]["total_tokens"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn test_chat_completion_empty_messages_returns_400() {
    let app = app();
    let body = serde_json::json!({
        "model": "gpt-4o",
        "messages": []
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_chat_completion_invalid_model_returns_400() {
    let app = app();
    let body = serde_json::json!({
        "model": "nonexistent-model",
        "messages": [{"role": "user", "content": "Hello"}]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["type"], "invalid_request_error");
}

#[tokio::test]
async fn test_chat_completion_invalid_temperature_returns_400() {
    let app = app();
    let body = serde_json::json!({
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "Hello"}],
        "temperature": 3.0
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_chat_completion_zero_max_tokens_returns_400() {
    let app = app();
    let body = serde_json::json!({
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "Hello"}],
        "max_tokens": 0
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ---- Request ID Propagation ----

#[tokio::test]
async fn test_request_id_echoed_in_response() {
    let app = app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .header("x-request-id", "test-req-123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("x-request-id").unwrap().to_str().unwrap(),
        "test-req-123"
    );
}

#[tokio::test]
async fn test_request_id_generated_when_missing() {
    let app = app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    // Should have a generated UUID
    let req_id = response.headers().get("x-request-id").unwrap().to_str().unwrap();
    assert!(!req_id.is_empty());
    assert!(req_id.contains('-')); // UUID format
}

// ---- Metrics Endpoint ----

#[tokio::test]
async fn test_metrics_endpoint() {
    let app = app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap().to_str().unwrap(),
        "text/plain; version=0.0.4; charset=utf-8"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("neurust_requests_total"));
    assert!(text.contains("neurust_errors_total"));
    assert!(text.contains("neurust_providers_registered"));
}

// ---- Auth Middleware ----

#[tokio::test]
async fn test_auth_rejects_invalid_key_when_configured() {
    let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MockProvider::openai())];
    let config = NeuRustConfig {
        server: ServerConfig {
            address: "127.0.0.1:0".to_string(),
            graceful_shutdown_sec: None,
            max_stream_buffer_tokens: None,
        },
        providers: vec![ProviderConfig {
            id: "mock-openai".to_string(),
            provider_type: "openai".to_string(),
            api_key_env: "FAKE".to_string(),
            base_url: None,
            models: vec!["gpt-4o".into()],
            priority: Some(1),
            max_connections: None,
        }],
        intelligence: None,
        security: None,
        observability: None,
        auth: Some(AuthConfig {
            api_keys: Some(vec![ApiKeyEntry {
                key: "sk-valid-key".to_string(),
                name: "test-client".to_string(),
                rate_limit: None,
            }]),
            jwt: None,
        }),
    };

    let (event_tx, _) = new_event_bus();
    let state = AppState::new(providers, config, event_tx);
    let app = build_router(state);

    // Request without key should be rejected
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "model": "gpt-4o",
                        "messages": [{"role": "user", "content": "Hello"}]
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_passes_with_valid_key() {
    let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MockProvider::openai())];
    let config = NeuRustConfig {
        server: ServerConfig {
            address: "127.0.0.1:0".to_string(),
            graceful_shutdown_sec: None,
            max_stream_buffer_tokens: None,
        },
        providers: vec![ProviderConfig {
            id: "mock-openai".to_string(),
            provider_type: "openai".to_string(),
            api_key_env: "FAKE".to_string(),
            base_url: None,
            models: vec!["gpt-4o".into()],
            priority: Some(1),
            max_connections: None,
        }],
        intelligence: None,
        security: None,
        observability: None,
        auth: Some(AuthConfig {
            api_keys: Some(vec![ApiKeyEntry {
                key: "sk-valid-key".to_string(),
                name: "test-client".to_string(),
                rate_limit: None,
            }]),
            jwt: None,
        }),
    };

    let (event_tx, _) = new_event_bus();
    let state = AppState::new(providers, config, event_tx);
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header("authorization", "Bearer sk-valid-key")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "model": "gpt-4o",
                        "messages": [{"role": "user", "content": "Hello"}]
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_health_endpoints_skip_auth() {
    let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MockProvider::openai())];
    let config = NeuRustConfig {
        server: ServerConfig {
            address: "127.0.0.1:0".to_string(),
            graceful_shutdown_sec: None,
            max_stream_buffer_tokens: None,
        },
        providers: vec![ProviderConfig {
            id: "mock-openai".to_string(),
            provider_type: "openai".to_string(),
            api_key_env: "FAKE".to_string(),
            base_url: None,
            models: vec!["gpt-4o".into()],
            priority: Some(1),
            max_connections: None,
        }],
        intelligence: None,
        security: None,
        observability: None,
        auth: Some(AuthConfig {
            api_keys: Some(vec![ApiKeyEntry {
                key: "sk-test".to_string(),
                name: "test".to_string(),
                rate_limit: None,
            }]),
            jwt: None,
        }),
    };

    let (event_tx, _) = new_event_bus();
    let state = AppState::new(providers, config, event_tx);
    let app = build_router(state);

    // Health endpoints should work without auth
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Metrics should also skip auth
    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
