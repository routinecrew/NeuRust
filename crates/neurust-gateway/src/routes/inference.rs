//! Chat completions endpoint (OpenAI-compatible).
//!
//! `POST /v1/chat/completions` — Supports both streaming (SSE) and non-streaming responses.

use std::collections::HashMap;
use std::convert::Infallible;
use std::pin::Pin;
use std::task::Context;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info};

use crate::contracts::{
    CostEvent, GatewayEvent, GatewayEventType, Message, ModelSpec, RequestContext, Role,
    StreamChunk, TokenUsage, UnifiedRequest,
};
use crate::middleware::request_id::RequestId;
use crate::routes::health::{inc_error_count, inc_request_count};
use crate::state::AppState;

/// Register inference routes.
pub fn routes() -> Router<AppState> {
    Router::new().route("/v1/chat/completions", post(chat_completions))
}

// ============================================================
// OpenAI-compatible request/response types
// ============================================================

#[derive(Debug, Deserialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<OpenAIMessage>,
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    stream: Option<bool>,
    /// Extra fields are passed through to the provider.
    #[serde(flatten)]
    extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct OpenAIMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ChatCompletionResponse {
    id: String,
    object: &'static str,
    created: u64,
    model: String,
    choices: Vec<Choice>,
    usage: UsageResponse,
}

#[derive(Serialize)]
struct Choice {
    index: u32,
    message: OpenAIMessage,
    finish_reason: String,
}

#[derive(Serialize)]
struct UsageResponse {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[derive(Serialize)]
struct StreamChunkResponse {
    id: String,
    object: &'static str,
    created: u64,
    model: String,
    choices: Vec<StreamChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<UsageResponse>,
}

#[derive(Serialize)]
struct StreamChoice {
    index: u32,
    delta: DeltaContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    finish_reason: Option<String>,
}

#[derive(Serialize)]
struct DeltaContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    message: String,
    r#type: String,
    code: Option<String>,
}

// ============================================================
// Helpers
// ============================================================

/// Unix timestamp in seconds (for OpenAI `created` field).
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before UNIX epoch")
        .as_secs()
}

/// Unix timestamp in milliseconds (for internal event bus).
fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before UNIX epoch")
        .as_millis() as u64
}

fn generate_id() -> String {
    format!("chatcmpl-{}", uuid::Uuid::new_v4().as_simple())
}

fn parse_role(s: &str) -> Role {
    match s {
        "system" => Role::System,
        "assistant" => Role::Assistant,
        _ => Role::User,
    }
}

fn to_unified_request(req: &ChatCompletionRequest) -> UnifiedRequest {
    let messages = req
        .messages
        .iter()
        .map(|m| Message {
            role: parse_role(&m.role),
            content: m.content.clone(),
        })
        .collect();

    UnifiedRequest {
        model: ModelSpec {
            model_name: req.model.clone(),
            provider_id: None,
        },
        messages,
        temperature: req.temperature,
        max_tokens: req.max_tokens,
        stream: req.stream.unwrap_or(false),
        context: RequestContext::default(),
        extra_params: req.extra.clone(),
    }
}

fn make_error_response(status: StatusCode, message: &str, error_type: &str) -> impl IntoResponse {
    let body = ErrorResponse {
        error: ErrorDetail {
            message: message.to_string(),
            r#type: error_type.to_string(),
            code: None,
        },
    };
    (status, Json(body))
}

/// Poll next item from a pinned stream using poll_fn.
async fn stream_next(
    stream: &mut Pin<Box<dyn futures_core::Stream<Item = anyhow::Result<StreamChunk>> + Send>>,
) -> Option<anyhow::Result<StreamChunk>> {
    std::future::poll_fn(|cx: &mut Context<'_>| stream.as_mut().poll_next(cx)).await
}

// ============================================================
// Handler
// ============================================================

/// POST /v1/chat/completions — OpenAI-compatible chat completions.
async fn chat_completions(
    State(state): State<AppState>,
    request_id: Option<axum::Extension<RequestId>>,
    Json(req): Json<ChatCompletionRequest>,
) -> impl IntoResponse {
    inc_request_count();

    let req_id = request_id
        .map(|r| r.0 .0.clone())
        .unwrap_or_else(|| "unknown".to_string());

    // Request validation
    if req.messages.is_empty() {
        return make_error_response(
            StatusCode::BAD_REQUEST,
            "messages must be a non-empty array",
            "invalid_request_error",
        )
        .into_response();
    }

    if let Some(t) = req.temperature {
        if !(0.0..=2.0).contains(&t) {
            return make_error_response(
                StatusCode::BAD_REQUEST,
                "temperature must be between 0 and 2",
                "invalid_request_error",
            )
            .into_response();
        }
    }

    if let Some(max_t) = req.max_tokens {
        if max_t == 0 {
            return make_error_response(
                StatusCode::BAD_REQUEST,
                "max_tokens must be greater than 0",
                "invalid_request_error",
            )
            .into_response();
        }
    }

    let is_stream = req.stream.unwrap_or(false);
    let model_name = req.model.clone();

    debug!(model = %model_name, stream = is_stream, request_id = %req_id, "Chat completion request");

    // Find a provider that supports this model
    let provider = match state.find_provider(&model_name) {
        Some(p) => p,
        None => {
            error!(model = %model_name, "No provider found for model");
            return make_error_response(
                StatusCode::BAD_REQUEST,
                &format!(
                    "The model '{}' does not exist. Check /v1/models for available models.",
                    model_name
                ),
                "invalid_request_error",
            )
            .into_response();
        }
    };

    let unified_req = to_unified_request(&req);

    // Check cache before calling provider (non-streaming only)
    if !is_stream {
        if let Some(ref cache) = state.cache {
            if let Some(cached_response) = cache.lookup(&unified_req) {
                debug!(model = %model_name, "Cache hit — returning cached response");

                // Publish cache hit event
                let _ = state.event_tx.send(GatewayEvent {
                    timestamp_ms: now_unix_ms(),
                    event_type: GatewayEventType::CacheHit {
                        cache_tier: "tiered".to_string(),
                        model: model_name.clone(),
                    },
                });

                return (
                    StatusCode::OK,
                    Json(ChatCompletionResponse {
                        id: generate_id(),
                        object: "chat.completion",
                        created: now_unix(),
                        model: cached_response.model,
                        choices: vec![Choice {
                            index: 0,
                            message: OpenAIMessage {
                                role: "assistant".to_string(),
                                content: cached_response.content,
                            },
                            finish_reason: "stop".to_string(),
                        }],
                        usage: UsageResponse {
                            prompt_tokens: cached_response.usage.prompt_tokens,
                            completion_tokens: cached_response.usage.completion_tokens,
                            total_tokens: cached_response.usage.total_tokens,
                        },
                    }),
                )
                    .into_response();
            }
        }
    }

    if is_stream {
        match handle_stream(&state, &provider, &unified_req).await {
            Ok(sse) => sse.into_response(),
            Err(e) => {
                inc_error_count();
                error!(error = %e, request_id = %req_id, "Streaming completion failed");
                make_error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("Streaming error: {}", e),
                    "server_error",
                )
                .into_response()
            }
        }
    } else {
        match handle_complete(&state, &provider, &unified_req).await {
            Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
            Err(e) => {
                inc_error_count();
                error!(error = %e, request_id = %req_id, "Completion failed");
                make_error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("Completion error: {}", e),
                    "server_error",
                )
                .into_response()
            }
        }
    }
}

/// Handle non-streaming completion.
async fn handle_complete(
    state: &AppState,
    provider: &std::sync::Arc<dyn crate::contracts::Provider>,
    request: &UnifiedRequest,
) -> anyhow::Result<ChatCompletionResponse> {
    let response = provider.complete(request).await?;

    info!(
        model = %response.model,
        provider = %response.provider_id,
        latency_ms = response.latency_ms,
        tokens = response.usage.total_tokens,
        "Completion finished"
    );

    // Store in cache
    if let Some(ref cache) = state.cache {
        cache.store(request, &response);
    }

    // Record cost event
    let cost_event = CostEvent {
        timestamp_ms: now_unix_ms(),
        provider_id: response.provider_id.clone(),
        model: response.model.clone(),
        prompt_tokens: response.usage.prompt_tokens,
        completion_tokens: response.usage.completion_tokens,
        cost_usd: 0.0, // CostTracker computes this
        latency_ms: response.latency_ms,
        cached: false,
    };
    let store = state.event_store.clone();
    tokio::spawn(async move {
        if let Err(e) = store.record_cost(&cost_event).await {
            tracing::warn!(error = %e, "Failed to record cost event");
        }
    });

    // Publish event (ignore send errors — no receivers is fine)
    let _ = state.event_tx.send(GatewayEvent {
        timestamp_ms: now_unix_ms(),
        event_type: GatewayEventType::RequestCompleted {
            provider_id: response.provider_id.clone(),
            model: response.model.clone(),
            latency_ms: response.latency_ms,
            tokens: response.usage.clone(),
            cached: false,
        },
    });

    Ok(ChatCompletionResponse {
        id: generate_id(),
        object: "chat.completion",
        created: now_unix(),
        model: response.model,
        choices: vec![Choice {
            index: 0,
            message: OpenAIMessage {
                role: "assistant".to_string(),
                content: response.content,
            },
            finish_reason: "stop".to_string(),
        }],
        usage: UsageResponse {
            prompt_tokens: response.usage.prompt_tokens,
            completion_tokens: response.usage.completion_tokens,
            total_tokens: response.usage.total_tokens,
        },
    })
}

/// Handle streaming completion — returns an SSE response.
async fn handle_stream(
    state: &AppState,
    provider: &std::sync::Arc<dyn crate::contracts::Provider>,
    request: &UnifiedRequest,
) -> anyhow::Result<Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>>> {
    let chunk_stream = provider.complete_stream(request).await?;
    let completion_id = generate_id();
    let model = request.model.model_name.clone();
    let created = now_unix();
    let event_tx = state.event_tx.clone();
    let provider_name = provider.name().to_string();
    let stream_start = std::time::Instant::now();

    let stream = async_stream::stream! {
        let mut pinned = chunk_stream;
        let mut first = true;

        loop {
            let item = stream_next(&mut pinned).await;

            match item {
                Some(Ok(chunk)) => {
                    let response = StreamChunkResponse {
                        id: completion_id.clone(),
                        object: "chat.completion.chunk",
                        created,
                        model: model.clone(),
                        choices: vec![StreamChoice {
                            index: 0,
                            delta: DeltaContent {
                                role: if first { Some("assistant".to_string()) } else { None },
                                content: if chunk.delta.is_empty() {
                                    None
                                } else {
                                    Some(chunk.delta.clone())
                                },
                            },
                            finish_reason: if chunk.finished {
                                Some("stop".to_string())
                            } else {
                                None
                            },
                        }],
                        usage: chunk.usage.as_ref().map(|u| UsageResponse {
                            prompt_tokens: u.prompt_tokens,
                            completion_tokens: u.completion_tokens,
                            total_tokens: u.total_tokens,
                        }),
                    };

                    first = false;

                    // Publish event BEFORE yielding final chunk (avoid race)
                    if chunk.finished {
                        let latency_ms = stream_start.elapsed().as_millis() as u64;
                        if let Some(usage) = &chunk.usage {
                            let _ = event_tx.send(GatewayEvent {
                                timestamp_ms: now_unix_ms(),
                                event_type: GatewayEventType::RequestCompleted {
                                    provider_id: provider_name.clone(),
                                    model: model.clone(),
                                    latency_ms,
                                    tokens: TokenUsage {
                                        prompt_tokens: usage.prompt_tokens,
                                        completion_tokens: usage.completion_tokens,
                                        total_tokens: usage.total_tokens,
                                    },
                                    cached: false,
                                },
                            });
                        }
                    }

                    match serde_json::to_string(&response) {
                        Ok(json) => {
                            yield Ok::<Event, Infallible>(Event::default().data(json));
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "Failed to serialize stream chunk");
                            break;
                        }
                    }

                    if chunk.finished {
                        break;
                    }
                }
                Some(Err(e)) => {
                    tracing::error!(error = %e, "Stream chunk error");
                    // Send an error event so the client knows something went wrong
                    let err_json = serde_json::json!({
                        "error": {
                            "message": format!("Stream error: {}", e),
                            "type": "server_error"
                        }
                    });
                    if let Ok(json) = serde_json::to_string(&err_json) {
                        yield Ok::<Event, Infallible>(Event::default().data(json));
                    }
                    break;
                }
                None => break,
            }
        }

        // Always send [DONE] marker per OpenAI spec
        yield Ok::<Event, Infallible>(Event::default().data("[DONE]"));
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
