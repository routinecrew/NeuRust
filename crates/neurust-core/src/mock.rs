// ============================================================
// 공용 Mock — 모든 에이전트가 독립 개발 시 사용
// ============================================================
// 정본(canonical source): contracts/mock.rs

use std::pin::Pin;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;

use crate::contracts::*;

// ============================================================
// MockProvider
// ============================================================

/// 테스트용 가짜 LLM 프로바이더
pub struct MockProvider {
    name: String,
    models: Vec<String>,
    latency_ms: u64,
}

impl MockProvider {
    /// 새 mock 프로바이더 생성
    pub fn new(name: &str, models: Vec<String>, latency_ms: u64) -> Self {
        Self {
            name: name.to_string(),
            models,
            latency_ms,
        }
    }

    /// OpenAI 호환 mock 프로바이더
    pub fn openai() -> Self {
        Self::new(
            "mock-openai",
            vec!["gpt-4o".into(), "gpt-4o-mini".into()],
            50,
        )
    }

    /// Anthropic mock 프로바이더
    pub fn anthropic() -> Self {
        Self::new(
            "mock-anthropic",
            vec!["claude-sonnet-4-20250514".into(), "claude-haiku-4-5-20251001".into()],
            80,
        )
    }
}

#[async_trait]
impl Provider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn supported_models(&self) -> Vec<String> {
        self.models.clone()
    }

    async fn complete(&self, request: &UnifiedRequest) -> Result<UnifiedResponse> {
        tokio::time::sleep(Duration::from_millis(self.latency_ms)).await;

        Ok(UnifiedResponse {
            content: format!("Mock response to: {}", request.last_user_message()),
            model: request.model.model_name.clone(),
            usage: TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
            },
            provider_id: self.name.clone(),
            latency_ms: self.latency_ms,
            upstream_id: Some("mock-id-001".to_string()),
        })
    }

    async fn complete_stream(
        &self,
        request: &UnifiedRequest,
    ) -> Result<Pin<Box<dyn futures_core::Stream<Item = Result<StreamChunk>> + Send>>> {
        let content = format!("Mock stream response to: {}", request.last_user_message());
        let words: Vec<String> = content.split_whitespace().map(String::from).collect();
        let latency = self.latency_ms;

        let stream = async_stream::stream! {
            for (i, word) in words.iter().enumerate() {
                tokio::time::sleep(Duration::from_millis(latency / 10)).await;
                let is_last = i == words.len() - 1;
                yield Ok(StreamChunk {
                    delta: if i == 0 { word.clone() } else { format!(" {}", word) },
                    finished: is_last,
                    usage: if is_last {
                        Some(TokenUsage {
                            prompt_tokens: 10,
                            completion_tokens: words.len() as u32,
                            total_tokens: 10 + words.len() as u32,
                        })
                    } else {
                        None
                    },
                });
            }
        };

        Ok(Box::pin(stream))
    }

    async fn health_check(&self) -> ProviderHealth {
        ProviderHealth {
            status: HealthStatus::Healthy,
            latency_ms: self.latency_ms,
            error: None,
        }
    }
}

// ============================================================
// MockEventStore
// ============================================================

/// 테스트용 인메모리 이벤트 저장소
pub struct MockEventStore {
    events: tokio::sync::RwLock<Vec<CostEvent>>,
}

impl MockEventStore {
    /// 빈 mock 저장소 생성
    pub fn new() -> Self {
        Self {
            events: tokio::sync::RwLock::new(Vec::new()),
        }
    }
}

impl Default for MockEventStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventStore for MockEventStore {
    async fn record_cost(&self, event: &CostEvent) -> Result<()> {
        self.events.write().await.push(event.clone());
        Ok(())
    }

    async fn daily_costs(&self, days: u32) -> Result<Vec<f64>> {
        Ok((0..days).map(|d| 10.0 + d as f64 * 0.5).collect())
    }

    async fn all_cost_events(&self) -> Result<Vec<CostEvent>> {
        Ok(self.events.read().await.clone())
    }

    async fn all_metrics(&self) -> Result<Vec<MetricEntry>> {
        Ok(Vec::new())
    }

    async fn ping(&self) -> Result<()> {
        Ok(())
    }
}

// ============================================================
// MockEventBus
// ============================================================

/// 테스트용 이벤트 버스 생성
pub fn mock_event_bus() -> EventSender {
    let (tx, _) = new_event_bus();
    tx
}
