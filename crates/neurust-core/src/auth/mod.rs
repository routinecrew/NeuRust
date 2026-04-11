use subtle::ConstantTimeEq;

use crate::contracts::AuthConfig;
use crate::error::NeuRustError;

/// API 키 인증 관리자
pub struct AuthManager {
    api_keys: Vec<ApiKeyInfo>,
}

struct ApiKeyInfo {
    key: String,
    name: String,
    #[allow(dead_code)]
    rate_limit: Option<u32>,
}

impl AuthManager {
    /// 설정에서 AuthManager 생성
    pub fn from_config(config: &Option<AuthConfig>) -> Self {
        let api_keys = config
            .as_ref()
            .and_then(|c| c.api_keys.as_ref())
            .map(|keys| {
                keys.iter()
                    .map(|k| ApiKeyInfo {
                        key: k.key.clone(),
                        name: k.name.clone(),
                        rate_limit: k.rate_limit,
                    })
                    .collect()
            })
            .unwrap_or_default();

        Self { api_keys }
    }

    /// API 키 검증 — 유효하면 키 이름 반환.
    /// 상수 시간 비교로 타이밍 공격 방지.
    pub fn validate_key(&self, key: &str) -> Result<String, NeuRustError> {
        if self.api_keys.is_empty() {
            return Ok("anonymous".to_string());
        }

        // 모든 키를 순회하여 타이밍 정보 유출 방지
        let mut matched_name: Option<String> = None;
        for k in &self.api_keys {
            if k.key.as_bytes().ct_eq(key.as_bytes()).into() {
                matched_name = Some(k.name.clone());
            }
        }

        matched_name.ok_or_else(|| {
            tracing::warn!("Invalid API key attempt (prefix: {}...)", &key[..key.len().min(4)]);
            NeuRustError::Auth("Invalid API key".to_string())
        })
    }

    /// 인증이 활성화되었는지 확인
    pub fn is_enabled(&self) -> bool {
        !self.api_keys.is_empty()
    }
}

impl Default for AuthManager {
    fn default() -> Self {
        Self::from_config(&None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{AuthConfig, ApiKeyEntry};

    #[test]
    fn test_no_keys_returns_anonymous() {
        let mgr = AuthManager::from_config(&None);
        assert_eq!(mgr.validate_key("anything").unwrap(), "anonymous");
    }

    #[test]
    fn test_valid_key() {
        let config = Some(AuthConfig {
            api_keys: Some(vec![ApiKeyEntry {
                key: "sk-test-123".to_string(),
                name: "test-client".to_string(),
                rate_limit: None,
            }]),
            jwt: None,
        });
        let mgr = AuthManager::from_config(&config);
        assert_eq!(mgr.validate_key("sk-test-123").unwrap(), "test-client");
    }

    #[test]
    fn test_invalid_key() {
        let config = Some(AuthConfig {
            api_keys: Some(vec![ApiKeyEntry {
                key: "sk-test-123".to_string(),
                name: "test-client".to_string(),
                rate_limit: None,
            }]),
            jwt: None,
        });
        let mgr = AuthManager::from_config(&config);
        assert!(mgr.validate_key("wrong-key").is_err());
    }

    #[test]
    fn test_is_enabled() {
        let mgr = AuthManager::from_config(&None);
        assert!(!mgr.is_enabled());

        let config = Some(AuthConfig {
            api_keys: Some(vec![ApiKeyEntry {
                key: "key".to_string(),
                name: "n".to_string(),
                rate_limit: None,
            }]),
            jwt: None,
        });
        let mgr2 = AuthManager::from_config(&config);
        assert!(mgr2.is_enabled());
    }
}
