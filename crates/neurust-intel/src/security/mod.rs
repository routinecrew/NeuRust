//! # Security Layer
//!
//! PII detection and masking, prompt injection defense, and audit logging.
//! Feature-gated under `security`.

pub mod pii_detector;
pub mod pii_masker;
pub mod prompt_guard;
pub mod audit_logger;
