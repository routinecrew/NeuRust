use std::sync::Arc;

use anyhow::Result;

use crate::contracts::*;
use crate::provider::ProviderRegistry;

/// 폴백 실행기 — 프로바이더 실패 시 다른 프로바이더로 재시도
pub struct FallbackExecutor {
    providers: Arc<ProviderRegistry>,
    max_retries: usize,
}

impl FallbackExecutor {
    /// 새 폴백 실행기 생성
    pub fn new(providers: Arc<ProviderRegistry>, max_retries: usize) -> Self {
        Self {
            providers,
            max_retries,
        }
    }

    /// 폴백 포함 비스트리밍 요청 실행
    pub async fn execute_with_fallback(
        &self,
        request: &UnifiedRequest,
    ) -> Result<UnifiedResponse> {
        let mut last_error = None;

        for (attempt, provider) in self.providers.all().iter().enumerate() {
            if attempt > self.max_retries {
                break;
            }

            if !provider
                .supported_models()
                .contains(&request.model.model_name)
            {
                continue;
            }

            match provider.complete(request).await {
                Ok(response) => {
                    if attempt > 0 {
                        tracing::info!(
                            provider = %provider.name(),
                            attempt = attempt + 1,
                            "Fallback succeeded"
                        );
                    }
                    return Ok(response);
                }
                Err(e) => {
                    tracing::warn!(
                        provider = %provider.name(),
                        attempt = attempt + 1,
                        error = %e,
                        "Provider failed, trying next"
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            anyhow::anyhow!("No providers available for model: {}", request.model.model_name)
        }))
    }
}
