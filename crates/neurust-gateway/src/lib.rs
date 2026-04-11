//! # neurust-gateway
//!
//! OpenAI-compatible HTTP API gateway for NeuRust.
//! Provides chat completions, model listing, and health check endpoints
//! using Axum with SSE streaming support.

pub mod contracts;
pub mod middleware;
pub mod mock;
pub mod routes;
pub mod server;
pub mod state;
