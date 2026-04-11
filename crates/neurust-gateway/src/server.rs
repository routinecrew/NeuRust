//! Axum HTTP server setup with graceful shutdown.

use std::time::Duration;

use axum::http::{header, Method};
use axum::middleware;
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::middleware::auth::auth_middleware;
use crate::middleware::request_id::request_id_middleware;
use crate::routes;
use crate::state::AppState;

/// Build the Axum router with all routes and middleware.
#[allow(deprecated)] // TimeoutLayer::new is deprecated but with_status_code requires a newer API
pub fn build_router(state: AppState) -> Router {
    // Production-safe CORS: restrict methods and headers
    let cors = CorsLayer::new()
        .allow_origin(tower_http::cors::Any) // TODO: configure allowed origins from config
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            header::HeaderName::from_static("x-api-key"),
        ])
        .max_age(Duration::from_secs(3600));

    let timeout =
        tower_http::timeout::TimeoutLayer::new(Duration::from_secs(120));

    let api_routes = routes::api_routes(state.clone());

    Router::new()
        .merge(api_routes)
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .layer(middleware::from_fn(request_id_middleware))
        .layer(cors)
        .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024)) // 10MB max request body
        .layer(timeout)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Start the Axum server with graceful shutdown.
pub async fn run(state: AppState) -> anyhow::Result<()> {
    let address = state.config.server.address.clone();
    let shutdown_sec = state.config.server.graceful_shutdown_sec.unwrap_or(30);

    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(&address).await?;
    info!(address = %address, "NeuRust gateway listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(shutdown_sec))
        .await?;

    info!("NeuRust gateway shut down gracefully");
    Ok(())
}

/// Wait for SIGINT or SIGTERM, then allow a grace period.
async fn shutdown_signal(grace_sec: u32) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => { info!("Received SIGINT"); }
        _ = terminate => { info!("Received SIGTERM"); }
    }

    if grace_sec > 0 {
        info!(seconds = grace_sec, "Starting graceful shutdown");
        tokio::time::sleep(Duration::from_secs(u64::from(grace_sec))).await;
    }
}
