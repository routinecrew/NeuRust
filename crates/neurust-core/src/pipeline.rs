use std::sync::Arc;

use anyhow::{Context, Result};

use crate::contracts::*;
use crate::provider::ProviderRegistry;

/// 미들웨어 파이프라인 — 레이어 체인을 거쳐 프로바이더로 요청 전달
pub struct Pipeline {
    layers: Vec<Arc<dyn PipelineLayer>>,
    providers: Arc<ProviderRegistry>,
}

impl Pipeline {
    /// 새 파이프라인 생성
    pub fn new(providers: Arc<ProviderRegistry>) -> Self {
        Self {
            layers: Vec::new(),
            providers,
        }
    }

    /// 레이어 추가
    pub fn add_layer(&mut self, layer: Arc<dyn PipelineLayer>) {
        tracing::debug!("Pipeline layer added: {}", layer.name());
        self.layers.push(layer);
    }

    /// 프로바이더 레지스트리 참조
    pub fn providers(&self) -> &Arc<ProviderRegistry> {
        &self.providers
    }

    /// 요청 실행 — 레이어 전처리 → 프로바이더 호출 → 레이어 후처리
    pub async fn execute(&self, request: &mut UnifiedRequest) -> Result<PipelineResult> {
        // 전처리: 모든 레이어의 on_request 호출
        for layer in &self.layers {
            layer
                .on_request(request)
                .await
                .with_context(|| format!("Pipeline layer '{}' failed on request", layer.name()))?;
        }

        // 프로바이더 선택
        let provider = if let Some(pid) = &request.model.provider_id {
            self.providers
                .get_by_name(pid)
                .with_context(|| format!("Provider '{}' not found", pid))?
        } else {
            self.providers
                .get_provider(&request.model.model_name)
                .with_context(|| {
                    format!("No provider for model '{}'", request.model.model_name)
                })?
        };

        // Dual-path: 스트리밍 vs 비스트리밍
        if request.stream {
            let chunks = provider
                .complete_stream(request)
                .await
                .with_context(|| {
                    format!("Stream request failed on provider '{}'", provider.name())
                })?;
            Ok(PipelineResult::Stream {
                chunks,
                on_complete: None,
            })
        } else {
            let mut response = provider.complete(request).await.with_context(|| {
                format!("Request failed on provider '{}'", provider.name())
            })?;

            // 후처리: 모든 레이어의 on_response 호출 (역순)
            for layer in self.layers.iter().rev() {
                layer
                    .on_response(request, &mut response)
                    .await
                    .with_context(|| {
                        format!("Pipeline layer '{}' failed on response", layer.name())
                    })?;
            }

            Ok(PipelineResult::Complete(response))
        }
    }
}
