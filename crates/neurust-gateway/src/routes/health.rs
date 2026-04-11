//! Health check and metrics endpoints.
//!
//! - `GET /health/live` — Liveness probe (always 200).
//! - `GET /health/ready` — Readiness probe (checks provider health concurrently).
//! - `GET /metrics` — Basic Prometheus-compatible metrics.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::contracts::HealthStatus;
use crate::state::AppState;

/// Global request counter for /metrics.
static REQUEST_COUNT: AtomicU64 = AtomicU64::new(0);
/// Global error counter for /metrics.
static ERROR_COUNT: AtomicU64 = AtomicU64::new(0);

/// Increment the global request counter.
pub fn inc_request_count() {
    REQUEST_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// Increment the global error counter.
pub fn inc_error_count() {
    ERROR_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// Register health and metrics routes.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/health/live", get(liveness))
        .route("/health/ready", get(readiness))
        .route("/metrics", get(metrics))
}

/// Liveness probe — always returns 200 OK.
async fn liveness() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}

#[derive(Serialize)]
struct ReadinessResponse {
    status: String,
    providers: Vec<ProviderStatus>,
}

#[derive(Serialize)]
struct ProviderStatus {
    name: String,
    status: String,
    latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Readiness probe — checks all provider health concurrently.
/// Returns 200 if at least one provider is healthy, 503 otherwise.
async fn readiness(State(state): State<AppState>) -> impl IntoResponse {
    let start = Instant::now();

    // Run all health checks concurrently
    let futures: Vec<_> = state
        .providers
        .iter()
        .map(|provider| {
            let provider = provider.clone();
            async move {
                let health = provider.health_check().await;
                (provider.name().to_string(), health)
            }
        })
        .collect();

    let results = futures::future::join_all(futures).await;

    let mut any_healthy = false;
    let mut provider_statuses = Vec::with_capacity(results.len());

    for (name, health) in results {
        if health.status == HealthStatus::Healthy {
            any_healthy = true;
        }
        provider_statuses.push(ProviderStatus {
            name,
            status: health.status.to_string(),
            latency_ms: health.latency_ms,
            error: health.error,
        });
    }

    tracing::debug!(
        providers = provider_statuses.len(),
        any_healthy,
        total_ms = start.elapsed().as_millis() as u64,
        "Readiness check completed"
    );

    let response = ReadinessResponse {
        status: if any_healthy {
            "ready".to_string()
        } else {
            "not_ready".to_string()
        },
        providers: provider_statuses,
    };

    let status_code = if any_healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (status_code, Json(response))
}

/// Prometheus-compatible metrics endpoint.
///
/// Returns plain text in Prometheus exposition format.
async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    let request_count = REQUEST_COUNT.load(Ordering::Relaxed);
    let error_count = ERROR_COUNT.load(Ordering::Relaxed);
    let provider_count = state.providers.len();

    let body = format!(
        "# HELP neurust_requests_total Total number of requests processed.\n\
         # TYPE neurust_requests_total counter\n\
         neurust_requests_total {request_count}\n\
         \n\
         # HELP neurust_errors_total Total number of errors.\n\
         # TYPE neurust_errors_total counter\n\
         neurust_errors_total {error_count}\n\
         \n\
         # HELP neurust_providers_registered Number of registered providers.\n\
         # TYPE neurust_providers_registered gauge\n\
         neurust_providers_registered {provider_count}\n"
    );

    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
}
