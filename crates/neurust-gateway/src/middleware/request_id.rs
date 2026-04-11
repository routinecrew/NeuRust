//! Request ID middleware.
//!
//! Extracts `X-Request-ID` from incoming requests or generates a new UUID v4.
//! The request ID is added to request extensions and echoed in the response header.

use axum::extract::Request;
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;

/// A request identifier propagated through the request lifecycle.
#[derive(Clone, Debug)]
pub struct RequestId(pub String);

/// Middleware that ensures every request has a unique ID.
///
/// If the incoming request has an `X-Request-ID` header, it is reused.
/// Otherwise, a new UUID v4 is generated.
/// The ID is inserted into request extensions and echoed in the response.
pub async fn request_id_middleware(mut req: Request, next: Next) -> Response {
    let id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    req.extensions_mut().insert(RequestId(id.clone()));

    let mut response = next.run(req).await;

    if let Ok(header_val) = HeaderValue::from_str(&id) {
        response.headers_mut().insert("x-request-id", header_val);
    }

    response
}
