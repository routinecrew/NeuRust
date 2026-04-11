pub mod fallback;
pub mod health;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use crate::contracts::*;
use crate::provider::ProviderRegistry;

/// 라운드로빈 로드 밸런서 — 같은 모델을 지원하는 프로바이더 간 분배
pub struct LoadBalancer {
    providers: Arc<ProviderRegistry>,
    counter: AtomicUsize,
}

impl LoadBalancer {
    /// 새 로드 밸런서 생성
    pub fn new(providers: Arc<ProviderRegistry>) -> Self {
        Self {
            providers,
            counter: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl PipelineLayer for LoadBalancer {
    async fn on_request(&self, request: &mut UnifiedRequest) -> anyhow::Result<()> {
        // provider_id가 이미 설정되었으면 스킵
        if request.model.provider_id.is_some() {
            return Ok(());
        }

        // 해당 모델을 지원하는 프로바이더 간 라운드로빈
        let matching: Vec<_> = self
            .providers
            .all()
            .iter()
            .filter(|p| p.supported_models().contains(&request.model.model_name))
            .cloned()
            .collect();

        if !matching.is_empty() {
            let idx = self.counter.fetch_add(1, Ordering::Relaxed) % matching.len();
            let selected = &matching[idx];
            request.model.provider_id = Some(selected.name().to_string());
            tracing::debug!(
                model = %request.model.model_name,
                provider = %selected.name(),
                index = idx,
                total = matching.len(),
                "LoadBalancer: round-robin routed"
            );
        }

        Ok(())
    }

    fn name(&self) -> &str {
        "load_balancer"
    }
}
