pub mod openai;
pub mod anthropic;

use std::sync::Arc;

use crate::contracts::{Provider, ProviderConfig};
use crate::config::resolve_api_key;

/// 프로바이더 레지스트리 — 등록된 프로바이더에서 모델 매칭
pub struct ProviderRegistry {
    providers: Vec<Arc<dyn Provider>>,
}

impl ProviderRegistry {
    /// 빈 레지스트리 생성
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// 설정 기반 레지스트리 초기화
    pub fn from_configs(configs: &[ProviderConfig]) -> Self {
        let mut registry = Self::new();
        for cfg in configs {
            match cfg.provider_type.as_str() {
                "openai" => {
                    if let Ok(key) = resolve_api_key(&cfg.api_key_env) {
                        let provider = openai::OpenAiProvider::new(
                            &cfg.id,
                            key,
                            cfg.base_url.clone(),
                            cfg.models.clone(),
                        );
                        registry.register(Arc::new(provider));
                        tracing::info!("Registered OpenAI provider: {}", cfg.id);
                    } else {
                        tracing::warn!(
                            "Skipping provider '{}': {} not set",
                            cfg.id,
                            cfg.api_key_env
                        );
                    }
                }
                "anthropic" => {
                    if let Ok(key) = resolve_api_key(&cfg.api_key_env) {
                        let provider = anthropic::AnthropicProvider::new(
                            &cfg.id,
                            key,
                            cfg.base_url.clone(),
                            cfg.models.clone(),
                        );
                        registry.register(Arc::new(provider));
                        tracing::info!("Registered Anthropic provider: {}", cfg.id);
                    } else {
                        tracing::warn!(
                            "Skipping provider '{}': {} not set",
                            cfg.id,
                            cfg.api_key_env
                        );
                    }
                }
                other => {
                    tracing::warn!("Unknown provider type: {}", other);
                }
            }
        }
        registry
    }

    /// 프로바이더 등록
    pub fn register(&mut self, provider: Arc<dyn Provider>) {
        self.providers.push(provider);
    }

    /// 모델을 지원하는 프로바이더 검색
    pub fn get_provider(&self, model: &str) -> Option<Arc<dyn Provider>> {
        self.providers
            .iter()
            .find(|p| p.supported_models().iter().any(|m| m == model))
            .cloned()
    }

    /// 이름으로 프로바이더 검색
    pub fn get_by_name(&self, name: &str) -> Option<Arc<dyn Provider>> {
        self.providers.iter().find(|p| p.name() == name).cloned()
    }

    /// 전체 프로바이더 목록
    pub fn all(&self) -> &[Arc<dyn Provider>] {
        &self.providers
    }

    /// 지원하는 전체 모델 목록
    pub fn all_models(&self) -> Vec<(String, String)> {
        self.providers
            .iter()
            .flat_map(|p| {
                let name = p.name().to_string();
                p.supported_models()
                    .into_iter()
                    .map(move |m| (m, name.clone()))
            })
            .collect()
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}
