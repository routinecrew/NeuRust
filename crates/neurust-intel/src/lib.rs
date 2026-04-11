//! # neurust-intel
//!
//! Intelligence layer for NeuRust gateway.
//! Provides caching, cost-optimized routing, security (PII masking, prompt guard),
//! observability (cost tracking, forecasting), and pluggable event stores.
//!
//! Feature-gated modules:
//! - `cache-exact` (default): Exact hash-based cache with LFU eviction and cache warming
//! - `cache-semantic`: Semantic similarity cache (requires fastembed + simsimd)
//! - `cost-router`: Cost-aware model routing with Multi-Armed Bandit
//! - `security`: PII detection/masking + prompt injection defense
//! - `observability` (default): Cost tracking and forecasting
//! - `store-sqlite`: SQLite-backed event store
//! - `store-postgres`: PostgreSQL-backed event store

pub mod contracts;
pub mod mock;

#[cfg(feature = "cache-exact")]
pub mod cache;

#[cfg(feature = "cost-router")]
pub mod router;

#[cfg(feature = "security")]
pub mod security;

#[cfg(feature = "observability")]
pub mod observability;

pub mod store;
