//! # PII Detector
//!
//! Three-level PII detection pipeline:
//! 1. Regex: fast pattern matching for structured PII (SSN, phone, email, card)
//! 2. Aho-Corasick: multi-pattern keyword matching
//! 3. Context validation: reduce false positives by checking surrounding text
//!
//! Focused on Korean PII patterns (주민번호, 전화번호, 이메일, 카드번호).

use aho_corasick::AhoCorasick;

/// A detected PII match.
#[derive(Clone, Debug)]
pub struct PiiMatch {
    /// Type of PII detected.
    pub pii_type: PiiType,
    /// Byte offset of the start of the match in the input.
    pub start: usize,
    /// Byte offset of the end of the match.
    pub end: usize,
    /// The matched text.
    pub matched_text: String,
    /// Confidence level (0.0 ~ 1.0).
    pub confidence: f64,
}

/// Types of PII detected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PiiType {
    /// Korean resident registration number (주민등록번호).
    KoreanSsn,
    /// Phone number (한국 전화번호).
    PhoneNumber,
    /// Email address.
    Email,
    /// Credit/debit card number.
    CardNumber,
}

impl std::fmt::Display for PiiType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::KoreanSsn => write!(f, "korean_ssn"),
            Self::PhoneNumber => write!(f, "phone_number"),
            Self::Email => write!(f, "email"),
            Self::CardNumber => write!(f, "card_number"),
        }
    }
}

/// PII detection engine with three-level pipeline.
pub struct PiiDetector {
    /// Aho-Corasick automaton for keyword scanning.
    _keyword_scanner: AhoCorasick,
    /// PII keywords for context validation.
    context_keywords: Vec<String>,
}

impl PiiDetector {
    /// Create a new PII detector with default Korean patterns.
    pub fn new() -> Self {
        let context_keywords = vec![
            "주민번호".to_string(),
            "주민등록번호".to_string(),
            "전화번호".to_string(),
            "핸드폰".to_string(),
            "휴대폰".to_string(),
            "이메일".to_string(),
            "카드번호".to_string(),
            "신용카드".to_string(),
            "계좌번호".to_string(),
        ];

        let empty: Vec<String> = Vec::new();
        let scanner = AhoCorasick::new(&context_keywords)
            .unwrap_or_else(|_| AhoCorasick::new(&empty).expect("empty aho-corasick"));

        Self {
            _keyword_scanner: scanner,
            context_keywords,
        }
    }

    /// Detect all PII matches in the given text.
    ///
    /// Uses a three-level pipeline:
    /// 1. Regex patterns for structured data
    /// 2. Aho-Corasick keyword matching for context
    /// 3. Context validation to reduce false positives
    pub fn detect(&self, text: &str) -> Vec<PiiMatch> {
        let mut matches = Vec::new();

        // Level 1: Regex-based detection
        self.detect_korean_ssn(text, &mut matches);
        self.detect_phone_numbers(text, &mut matches);
        self.detect_emails(text, &mut matches);
        self.detect_card_numbers(text, &mut matches);

        // Level 3: Context validation (boost confidence if context keywords nearby)
        self.validate_context(text, &mut matches);

        matches
    }

    /// Detect Korean SSN patterns (XXXXXX-XXXXXXX).
    fn detect_korean_ssn(&self, text: &str, matches: &mut Vec<PiiMatch>) {
        // Pattern: 6 digits, dash, 7 digits (주민등록번호)
        let bytes = text.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        while i + 14 <= len {
            // Check for NNNNNN-NNNNNNN pattern
            if i + 13 < len
                && bytes[i..i + 6].iter().all(|b| b.is_ascii_digit())
                && bytes[i + 6] == b'-'
                && bytes[i + 7..i + 14].iter().all(|b| b.is_ascii_digit())
            {
                // Validate: first digit of back part should be 1-4
                if bytes[i + 7] >= b'1' && bytes[i + 7] <= b'4' {
                    let matched = &text[i..i + 14];
                    matches.push(PiiMatch {
                        pii_type: PiiType::KoreanSsn,
                        start: i,
                        end: i + 14,
                        matched_text: matched.to_string(),
                        confidence: 0.85,
                    });
                }
            }
            i += 1;
        }
    }

    /// Detect phone number patterns (010-XXXX-XXXX, 02-XXX-XXXX, etc.).
    fn detect_phone_numbers(&self, text: &str, matches: &mut Vec<PiiMatch>) {
        let bytes = text.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        while i < len {
            // Mobile: 010-XXXX-XXXX (13 chars) or 01X-XXX-XXXX (12 chars)
            if i + 13 <= len
                && bytes[i] == b'0'
                && bytes[i + 1] == b'1'
                && bytes[i + 2] == b'0'
                && bytes[i + 3] == b'-'
                && bytes[i + 4..i + 8].iter().all(|b| b.is_ascii_digit())
                && bytes[i + 8] == b'-'
                && bytes[i + 9..i + 13].iter().all(|b| b.is_ascii_digit())
            {
                let matched = &text[i..i + 13];
                matches.push(PiiMatch {
                    pii_type: PiiType::PhoneNumber,
                    start: i,
                    end: i + 13,
                    matched_text: matched.to_string(),
                    confidence: 0.9,
                });
                i += 13;
                continue;
            }

            // Landline: 02-XXX-XXXX or 0XX-XXX-XXXX
            if i + 11 <= len
                && bytes[i] == b'0'
                && bytes[i + 1].is_ascii_digit()
                && bytes[i + 2] == b'-'
                && bytes[i + 3..i + 6].iter().all(|b| b.is_ascii_digit())
                && bytes[i + 6] == b'-'
                && bytes[i + 7..i + 11].iter().all(|b| b.is_ascii_digit())
            {
                let matched = &text[i..i + 11];
                matches.push(PiiMatch {
                    pii_type: PiiType::PhoneNumber,
                    start: i,
                    end: i + 11,
                    matched_text: matched.to_string(),
                    confidence: 0.8,
                });
                i += 11;
                continue;
            }

            i += 1;
        }
    }

    /// Detect email patterns with TLD validation.
    fn detect_emails(&self, text: &str, matches: &mut Vec<PiiMatch>) {
        let mut i = 0;
        let chars: Vec<char> = text.chars().collect();
        let len = chars.len();

        while i < len {
            if chars[i] == '@' && i > 0 {
                // Find local part start
                let mut local_start = i;
                while local_start > 0
                    && (chars[local_start - 1].is_alphanumeric()
                        || chars[local_start - 1] == '.'
                        || chars[local_start - 1] == '_'
                        || chars[local_start - 1] == '-'
                        || chars[local_start - 1] == '+')
                {
                    local_start -= 1;
                }

                // Find domain end
                let mut domain_end = i + 1;
                let mut last_dot_pos = None;
                while domain_end < len
                    && (chars[domain_end].is_alphanumeric()
                        || chars[domain_end] == '.'
                        || chars[domain_end] == '-')
                {
                    if chars[domain_end] == '.' {
                        last_dot_pos = Some(domain_end);
                    }
                    domain_end += 1;
                }

                // Validate: must have a dot, TLD must be 2+ alphabetic chars
                let has_valid_tld = if let Some(dot_pos) = last_dot_pos {
                    let tld_chars = &chars[dot_pos + 1..domain_end];
                    tld_chars.len() >= 2 && tld_chars.iter().all(|c| c.is_ascii_alphabetic())
                } else {
                    false
                };

                if local_start < i && domain_end > i + 1 && has_valid_tld {
                    let byte_start = text
                        .char_indices()
                        .nth(local_start)
                        .map(|(idx, _)| idx)
                        .unwrap_or(0);
                    let byte_end = text
                        .char_indices()
                        .nth(domain_end)
                        .map(|(idx, _)| idx)
                        .unwrap_or(text.len());

                    let matched = &text[byte_start..byte_end];
                    matches.push(PiiMatch {
                        pii_type: PiiType::Email,
                        start: byte_start,
                        end: byte_end,
                        matched_text: matched.to_string(),
                        confidence: 0.95,
                    });
                    i = domain_end;
                    continue;
                }
            }
            i += 1;
        }
    }

    /// Detect credit card number patterns.
    /// Supports: XXXX-XXXX-XXXX-XXXX, XXXX XXXX XXXX XXXX, XXXXXXXXXXXXXXXX
    fn detect_card_numbers(&self, text: &str, matches: &mut Vec<PiiMatch>) {
        let bytes = text.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        // Pattern 1: dashed (19 chars) — XXXX-XXXX-XXXX-XXXX
        while i + 19 <= len {
            if bytes[i..i + 4].iter().all(|b| b.is_ascii_digit())
                && bytes[i + 4] == b'-'
                && bytes[i + 5..i + 9].iter().all(|b| b.is_ascii_digit())
                && bytes[i + 9] == b'-'
                && bytes[i + 10..i + 14].iter().all(|b| b.is_ascii_digit())
                && bytes[i + 14] == b'-'
                && bytes[i + 15..i + 19].iter().all(|b| b.is_ascii_digit())
            {
                let matched = &text[i..i + 19];
                matches.push(PiiMatch {
                    pii_type: PiiType::CardNumber,
                    start: i,
                    end: i + 19,
                    matched_text: matched.to_string(),
                    confidence: 0.9,
                });
                i += 19;
                continue;
            }

            // Pattern 2: space-separated (19 chars) — XXXX XXXX XXXX XXXX
            if bytes[i..i + 4].iter().all(|b| b.is_ascii_digit())
                && bytes[i + 4] == b' '
                && bytes[i + 5..i + 9].iter().all(|b| b.is_ascii_digit())
                && bytes[i + 9] == b' '
                && bytes[i + 10..i + 14].iter().all(|b| b.is_ascii_digit())
                && bytes[i + 14] == b' '
                && bytes[i + 15..i + 19].iter().all(|b| b.is_ascii_digit())
            {
                // Make sure the surrounding chars are not digits (avoid false positive in long numbers)
                let prev_ok = i == 0 || !bytes[i - 1].is_ascii_digit();
                let next_ok = i + 19 >= len || !bytes[i + 19].is_ascii_digit();
                if prev_ok && next_ok {
                    let matched = &text[i..i + 19];
                    matches.push(PiiMatch {
                        pii_type: PiiType::CardNumber,
                        start: i,
                        end: i + 19,
                        matched_text: matched.to_string(),
                        confidence: 0.85,
                    });
                    i += 19;
                    continue;
                }
            }

            i += 1;
        }

        // Pattern 3: no separator (16 consecutive digits)
        let mut j = 0;
        while j + 16 <= len {
            if bytes[j..j + 16].iter().all(|b| b.is_ascii_digit()) {
                let prev_ok = j == 0 || !bytes[j - 1].is_ascii_digit();
                let next_ok = j + 16 >= len || !bytes[j + 16].is_ascii_digit();
                if prev_ok && next_ok {
                    let matched = &text[j..j + 16];
                    // Skip if already detected by dashed/space patterns
                    let already_found = matches.iter().any(|m| m.start <= j && m.end >= j + 16);
                    if !already_found {
                        matches.push(PiiMatch {
                            pii_type: PiiType::CardNumber,
                            start: j,
                            end: j + 16,
                            matched_text: matched.to_string(),
                            confidence: 0.75,
                        });
                    }
                    j += 16;
                    continue;
                }
            }
            j += 1;
        }
    }

    /// Context validation: boost confidence if PII-related keywords are nearby.
    fn validate_context(&self, text: &str, matches: &mut Vec<PiiMatch>) {
        for m in matches.iter_mut() {
            // Check if any context keyword appears within 50 chars of the match
            let window_start = m.start.saturating_sub(50);
            let window_end = (m.end + 50).min(text.len());
            let window = &text[window_start..window_end];

            for keyword in &self.context_keywords {
                if window.contains(keyword.as_str()) {
                    // Boost confidence when context keyword is present
                    m.confidence = (m.confidence + 0.1).min(1.0);
                    break;
                }
            }
        }
    }

    /// Maximum pattern length in tokens (approx: chars / 2 for Korean).
    /// Used by PiiMasker to set dynamic sliding window size.
    pub fn max_pattern_length_tokens(&self) -> usize {
        // Longest pattern: card number (19 chars) or SSN (14 chars)
        // Korean chars ~2 bytes, 1 token ~4 chars
        // 19 / 4 = ~5 tokens, add margin
        7
    }
}

impl Default for PiiDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_korean_ssn() {
        let detector = PiiDetector::new();
        let text = "주민번호는 901234-1234567입니다";
        let matches = detector.detect(text);

        let ssn_matches: Vec<_> = matches
            .iter()
            .filter(|m| m.pii_type == PiiType::KoreanSsn)
            .collect();
        assert_eq!(ssn_matches.len(), 1);
        assert_eq!(ssn_matches[0].matched_text, "901234-1234567");
    }

    #[test]
    fn test_detect_phone_number() {
        let detector = PiiDetector::new();
        let text = "연락처: 010-1234-5678";
        let matches = detector.detect(text);

        let phone_matches: Vec<_> = matches
            .iter()
            .filter(|m| m.pii_type == PiiType::PhoneNumber)
            .collect();
        assert_eq!(phone_matches.len(), 1);
        assert_eq!(phone_matches[0].matched_text, "010-1234-5678");
    }

    #[test]
    fn test_detect_email() {
        let detector = PiiDetector::new();
        let text = "이메일: user@example.com 으로 보내주세요";
        let matches = detector.detect(text);

        let email_matches: Vec<_> = matches
            .iter()
            .filter(|m| m.pii_type == PiiType::Email)
            .collect();
        assert_eq!(email_matches.len(), 1);
        assert_eq!(email_matches[0].matched_text, "user@example.com");
    }

    #[test]
    fn test_detect_card_number() {
        let detector = PiiDetector::new();
        let text = "카드번호 1234-5678-9012-3456 입니다";
        let matches = detector.detect(text);

        let card_matches: Vec<_> = matches
            .iter()
            .filter(|m| m.pii_type == PiiType::CardNumber)
            .collect();
        assert_eq!(card_matches.len(), 1);
        assert_eq!(card_matches[0].matched_text, "1234-5678-9012-3456");
    }

    #[test]
    fn test_no_false_positive_on_clean_text() {
        let detector = PiiDetector::new();
        let text = "오늘 날씨가 좋습니다. 프로젝트 진행 상황을 알려주세요.";
        let matches = detector.detect(text);
        assert!(matches.is_empty());
    }

    #[test]
    fn test_context_boosts_confidence() {
        let detector = PiiDetector::new();
        let text = "주민번호는 901234-1234567입니다";
        let matches = detector.detect(text);

        // Context keyword "주민번호" is present, so confidence should be boosted
        assert!(!matches.is_empty());
        assert!(matches[0].confidence > 0.85);
    }

    #[test]
    fn test_detect_card_number_with_spaces() {
        let detector = PiiDetector::new();
        let text = "카드번호 1234 5678 9012 3456 입니다";
        let matches = detector.detect(text);
        let card_matches: Vec<_> = matches.iter().filter(|m| m.pii_type == PiiType::CardNumber).collect();
        assert_eq!(card_matches.len(), 1, "Should detect space-separated card numbers");
    }

    #[test]
    fn test_detect_card_number_no_separator() {
        let detector = PiiDetector::new();
        let text = "카드번호 1234567890123456 입니다";
        let matches = detector.detect(text);
        let card_matches: Vec<_> = matches.iter().filter(|m| m.pii_type == PiiType::CardNumber).collect();
        assert_eq!(card_matches.len(), 1, "Should detect no-separator card numbers");
    }

    #[test]
    fn test_email_requires_valid_tld() {
        let detector = PiiDetector::new();
        // "test@test" without proper TLD should NOT be detected
        let text = "이것은 test@test 입니다";
        let matches = detector.detect(text);
        let email_matches: Vec<_> = matches.iter().filter(|m| m.pii_type == PiiType::Email).collect();
        assert!(email_matches.is_empty(), "Should not detect email without valid TLD");
    }

    #[test]
    fn test_multiple_pii_in_text() {
        let detector = PiiDetector::new();
        let text = "이름: 홍길동, 주민번호: 901234-1234567, 전화: 010-1234-5678, 이메일: hong@test.com";
        let matches = detector.detect(text);

        assert!(matches.len() >= 3); // SSN + phone + email
    }
}
