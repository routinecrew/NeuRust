//! # PII Masker (v3)
//!
//! Bidirectional PII masking for requests and responses.
//! Implements adaptive sliding buffer for streaming with dynamic window
//! size based on the longest PII pattern.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tracing;

use crate::contracts::{PipelineLayer, UnifiedRequest, UnifiedResponse};
use super::audit_logger::AuditLogger;
use super::pii_detector::{PiiDetector, PiiMatch};

/// Direction of PII masking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Mask PII in requests only.
    Request,
    /// Mask PII in responses only.
    Response,
    /// Mask PII in both directions.
    Bidirectional,
}

impl Direction {
    /// Parse from config string.
    pub fn from_str_config(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "request" => Self::Request,
            "response" => Self::Response,
            _ => Self::Bidirectional,
        }
    }
}

/// PII masking pipeline layer.
///
/// Scans text for PII patterns and replaces them with masked versions.
/// Supports bidirectional masking (request + response) and adaptive
/// streaming window.
pub struct PiiMasker {
    /// PII detection engine.
    detector: Arc<PiiDetector>,
    /// Masking direction.
    direction: Direction,
    /// Audit logger for recording masking events.
    audit_logger: Arc<AuditLogger>,
    /// Dynamic window size in tokens for streaming PII detection.
    min_window_tokens: usize,
}

impl PiiMasker {
    /// Create a new PII masker.
    pub fn new(
        detector: Arc<PiiDetector>,
        direction: Direction,
        audit_logger: Arc<AuditLogger>,
    ) -> Self {
        // v3: Dynamic window size based on longest PII pattern
        let min_window_tokens = detector.max_pattern_length_tokens() + 20;

        Self {
            detector,
            direction,
            audit_logger,
            min_window_tokens,
        }
    }

    /// Create from config with auto window size.
    pub fn from_config(
        detector: Arc<PiiDetector>,
        direction_str: Option<&str>,
        min_window_tokens_str: Option<&str>,
    ) -> Self {
        let direction = Direction::from_str_config(direction_str.unwrap_or("bidirectional"));
        let audit_logger = Arc::new(AuditLogger::new());

        let min_window = match min_window_tokens_str {
            Some("auto") | None => detector.max_pattern_length_tokens() + 20,
            Some(s) => s.parse().unwrap_or(50),
        };

        Self {
            detector,
            direction,
            audit_logger,
            min_window_tokens: min_window,
        }
    }

    /// Mask all detected PII in the given text.
    pub fn mask_text(&self, text: &str) -> String {
        let matches = self.detector.detect(text);
        if matches.is_empty() {
            return text.to_string();
        }

        self.audit_logger
            .log_pii_detection(&matches, "text_masking");

        apply_masks(text, &matches)
    }

    /// Get the dynamic window size in tokens for streaming.
    pub fn window_size_tokens(&self) -> usize {
        self.min_window_tokens
    }
}

/// Apply masks to text, replacing PII matches with asterisks.
fn apply_masks(text: &str, matches: &[PiiMatch]) -> String {
    if matches.is_empty() {
        return text.to_string();
    }

    // Sort matches by start position
    let mut sorted: Vec<&PiiMatch> = matches.iter().collect();
    sorted.sort_by_key(|m| m.start);

    let mut result = String::with_capacity(text.len());
    let mut last_end = 0;

    for m in sorted {
        if m.start >= last_end {
            result.push_str(&text[last_end..m.start]);
            // Replace with type-specific mask
            let mask = match m.pii_type {
                super::pii_detector::PiiType::KoreanSsn => "******-*******",
                super::pii_detector::PiiType::PhoneNumber => "***-****-****",
                super::pii_detector::PiiType::Email => "****@****.***",
                super::pii_detector::PiiType::CardNumber => "****-****-****-****",
            };
            result.push_str(mask);
            last_end = m.end;
        }
    }

    result.push_str(&text[last_end..]);
    result
}

#[async_trait]
impl PipelineLayer for PiiMasker {
    fn name(&self) -> &str {
        "pii-masker"
    }

    async fn on_request(&self, request: &mut UnifiedRequest) -> Result<()> {
        if self.direction == Direction::Response {
            return Ok(());
        }

        let mut masked_any = false;
        for msg in &mut request.messages {
            let masked = self.mask_text(&msg.content);
            if masked != msg.content {
                masked_any = true;
                msg.content = masked;
            }
        }

        if masked_any {
            tracing::info!("PII masked in request messages");
        }
        Ok(())
    }

    async fn on_response(
        &self,
        _request: &UnifiedRequest,
        response: &mut UnifiedResponse,
    ) -> Result<()> {
        if self.direction == Direction::Request {
            return Ok(());
        }

        let masked = self.mask_text(&response.content);
        if masked != response.content {
            tracing::info!("PII masked in response");
            response.content = masked;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mask_korean_ssn() {
        let detector = Arc::new(PiiDetector::new());
        let masker = PiiMasker::new(
            detector,
            Direction::Bidirectional,
            Arc::new(AuditLogger::new()),
        );

        let text = "주민번호는 901234-1234567입니다";
        let masked = masker.mask_text(text);
        assert!(masked.contains("******-*******"));
        assert!(!masked.contains("901234-1234567"));
    }

    #[test]
    fn test_mask_phone() {
        let detector = Arc::new(PiiDetector::new());
        let masker = PiiMasker::new(
            detector,
            Direction::Bidirectional,
            Arc::new(AuditLogger::new()),
        );

        let text = "전화번호: 010-1234-5678";
        let masked = masker.mask_text(text);
        assert!(!masked.contains("010-1234-5678"));
    }

    #[test]
    fn test_mask_email() {
        let detector = Arc::new(PiiDetector::new());
        let masker = PiiMasker::new(
            detector,
            Direction::Bidirectional,
            Arc::new(AuditLogger::new()),
        );

        let text = "이메일: user@example.com 입니다";
        let masked = masker.mask_text(text);
        assert!(!masked.contains("user@example.com"));
    }

    #[test]
    fn test_no_masking_clean_text() {
        let detector = Arc::new(PiiDetector::new());
        let masker = PiiMasker::new(
            detector,
            Direction::Bidirectional,
            Arc::new(AuditLogger::new()),
        );

        let text = "안녕하세요, 오늘 날씨가 좋습니다";
        let masked = masker.mask_text(text);
        assert_eq!(masked, text);
    }

    #[test]
    fn test_direction_request_only() {
        let detector = Arc::new(PiiDetector::new());
        let masker = PiiMasker::new(
            detector,
            Direction::Request,
            Arc::new(AuditLogger::new()),
        );
        assert_eq!(masker.direction, Direction::Request);
    }
}
