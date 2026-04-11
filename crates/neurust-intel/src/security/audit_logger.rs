//! # Audit Logger
//!
//! Logs security events: PII detection, prompt injection attempts,
//! and output validation failures.

use tracing;

use super::pii_detector::PiiMatch;

/// Security audit logger.
///
/// Records security-relevant events for compliance and debugging.
pub struct AuditLogger {
    _enabled: bool,
}

impl AuditLogger {
    /// Create a new audit logger.
    pub fn new() -> Self {
        Self { _enabled: true }
    }

    /// Log PII detection events.
    pub fn log_pii_detection(&self, matches: &[PiiMatch], context: &str) {
        for m in matches {
            tracing::info!(
                pii_type = %m.pii_type,
                confidence = m.confidence,
                context = context,
                "PII detected and masked"
            );
        }
    }

    /// Log a prompt injection attempt.
    pub fn log_injection_attempt(&self, pattern: &str, source: &str) {
        tracing::warn!(
            pattern = pattern,
            source = source,
            "Prompt injection attempt detected"
        );
    }

    /// Log output validation failure (system prompt leakage).
    pub fn log_output_leak(&self, overlap_ratio: f32, threshold: f32) {
        tracing::warn!(
            overlap_ratio = overlap_ratio,
            threshold = threshold,
            "System prompt leakage detected in output"
        );
    }

    /// Log a streaming PII masking event.
    pub fn log_stream_masking(&self, pii_count: usize) {
        tracing::debug!(
            pii_matches = pii_count,
            "PII masked in streaming chunk"
        );
    }
}

impl Default for AuditLogger {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::pii_detector::PiiType;

    #[test]
    fn test_audit_logger_creation() {
        let logger = AuditLogger::new();
        // Should not panic
        logger.log_injection_attempt("test pattern", "test source");
        logger.log_output_leak(0.5, 0.3);
        logger.log_stream_masking(2);
    }

    #[test]
    fn test_log_pii_detection() {
        let logger = AuditLogger::new();
        let matches = vec![PiiMatch {
            pii_type: PiiType::Email,
            start: 0,
            end: 16,
            matched_text: "user@example.com".to_string(),
            confidence: 0.95,
        }];
        logger.log_pii_detection(&matches, "test");
    }
}
