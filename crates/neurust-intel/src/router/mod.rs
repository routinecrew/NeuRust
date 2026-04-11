//! # Cost Router
//!
//! Cost-aware model routing using weighted scoring and Multi-Armed Bandit
//! exploration. Feature-gated under `cost-router`.

pub mod cost_auctioneer;
pub mod bandit;
pub mod price_db;
pub mod complexity_gate;
