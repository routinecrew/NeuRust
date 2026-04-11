//! Model listing endpoint (OpenAI-compatible).
//!
//! `GET /v1/models` — Returns all available models in OpenAI list format.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::state::AppState;

/// Register model listing routes.
pub fn routes() -> Router<AppState> {
    Router::new().route("/v1/models", get(list_models))
}

#[derive(Serialize)]
struct ModelsResponse {
    object: &'static str,
    data: Vec<ModelObject>,
}

#[derive(Serialize)]
struct ModelObject {
    id: String,
    object: &'static str,
    created: u64,
    owned_by: String,
}

/// List all available models across registered providers.
async fn list_models(State(state): State<AppState>) -> impl IntoResponse {
    let models: Vec<ModelObject> = state
        .all_models()
        .into_iter()
        .map(|(model_id, provider_name)| ModelObject {
            id: model_id,
            object: "model",
            created: 0,
            owned_by: provider_name,
        })
        .collect();

    Json(ModelsResponse {
        object: "list",
        data: models,
    })
}
