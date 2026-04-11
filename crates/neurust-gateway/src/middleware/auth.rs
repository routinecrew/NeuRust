//! Authentication middleware.
//!
//! Extracts Bearer token or `x-api-key` header and validates against
//! configured API keys. If no auth config is present, all requests pass through.

use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use subtle::ConstantTimeEq;
use tracing::{debug, warn};

use crate::contracts::ApiKeyEntry;

/// Client identity attached to request extensions after successful auth.
#[derive(Clone, Debug)]
pub struct ClientIdentity {
    /// The name associated with the API key.
    pub name: String,
    /// Rate limit (requests per minute), if configured.
    pub rate_limit: Option<u32>,
}

#[derive(Serialize)]
struct AuthErrorResponse {
    error: AuthErrorDetail,
}

#[derive(Serialize)]
struct AuthErrorDetail {
    message: String,
    r#type: String,
}

fn auth_error(message: &str) -> Response {
    let body = AuthErrorResponse {
        error: AuthErrorDetail {
            message: message.to_string(),
            r#type: "authentication_error".to_string(),
        },
    };
    (StatusCode::UNAUTHORIZED, Json(body)).into_response()
}

/// Auth middleware function. Use with `axum::middleware::from_fn_with_state`.
pub async fn auth_middleware(
    axum::extract::State(state): axum::extract::State<crate::state::AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let api_keys = state
        .config
        .auth
        .as_ref()
        .and_then(|a| a.api_keys.as_ref())
        .cloned()
        .unwrap_or_default();

    // If no API keys configured, skip auth (open access)
    if api_keys.is_empty() {
        debug!("No API keys configured, skipping auth");
        return next.run(req).await;
    }

    // Skip auth for health endpoints (exact path match only)
    let path = req.uri().path();
    if matches!(path, "/health/live" | "/health/ready" | "/metrics") {
        return next.run(req).await;
    }

    // Extract token from Authorization header or x-api-key
    let token = extract_token(&req);

    match token {
        Some(t) => {
            // Constant-time comparison to prevent timing attacks.
            // Iterate all keys to avoid leaking which key matched via timing.
            let mut matched: Option<&ApiKeyEntry> = None;
            for key_entry in &api_keys {
                if key_entry.key.as_bytes().ct_eq(t.as_bytes()).into() {
                    matched = Some(key_entry);
                }
            }

            if let Some(key_entry) = matched {
                debug!(client = %key_entry.name, "Auth successful");
                req.extensions_mut().insert(ClientIdentity {
                    name: key_entry.name.clone(),
                    rate_limit: key_entry.rate_limit,
                });
                next.run(req).await
            } else {
                warn!("Invalid API key provided");
                auth_error("Invalid API key. Check your authentication credentials.")
            }
        }
        None => {
            warn!("No API key provided");
            auth_error(
                "Missing API key. Provide via 'Authorization: Bearer <key>' or 'x-api-key' header.",
            )
        }
    }
}

/// Extract API key from Authorization Bearer header or x-api-key header.
fn extract_token(req: &Request) -> Option<String> {
    // Try Authorization: Bearer <token>
    if let Some(auth_header) = req.headers().get(header::AUTHORIZATION) {
        if let Ok(value) = auth_header.to_str() {
            if let Some(token) = value.strip_prefix("Bearer ") {
                return Some(token.trim().to_string());
            }
        }
    }

    // Try x-api-key header
    if let Some(api_key_header) = req.headers().get("x-api-key") {
        if let Ok(value) = api_key_header.to_str() {
            return Some(value.trim().to_string());
        }
    }

    None
}
