//! Route definitions for the NeuRust gateway.

pub mod health;
pub mod inference;
pub mod models;

use axum::Router;

use crate::state::AppState;

/// Combine all API routes into a single router.
pub fn api_routes(_state: AppState) -> Router<AppState> {
    Router::new()
        .merge(inference::routes())
        .merge(health::routes())
        .merge(models::routes())
}
