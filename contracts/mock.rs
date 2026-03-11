// ============================================================
// 공용 Mock — 모든 에이전트가 독립 개발 시 사용
// ============================================================
// Agent A(core)가 완성되기 전에 B, C, D, E가 사용하는 가짜 구현.
// 실제 통합 시에는 이 파일을 제거하고 neurust-core의 실제 구현을 사용한다.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::contracts::*;

// ============================================================
// MockProvider — 가짜 LLM 프로바이더
// ============================================================

pub struct MockProvider {
    name: String,
    models: Vec<String>,
    latency_ms: u64,
}

impl MockProvider {
    pub fn new(name: &str, models: Vec<String>, latency_ms: u64) -> Self {
        Self {
            name: name.to_string(),
            models,
            latency_ms,
        }
    }

    /// OpenAI 호환 mock 프로바이더 생성
    pub fn openai() -> Self {
        Self::new(
            "mock-openai",
            vec!["gpt-4o".into(), "gpt-4o-mini".into()],
            50,
        )
    }

    /// Anthropic mock 프로바이더 생성
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
            content: format!(
                "Mock response to: {}",
                request.last_user_message()
            ),
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
// MockEventStore — 가짜 이벤트 저장소
// ============================================================

pub struct MockEventStore {
    events: tokio::sync::RwLock<Vec<CostEvent>>,
}

impl MockEventStore {
    pub fn new() -> Self {
        Self {
            events: tokio::sync::RwLock::new(Vec::new()),
        }
    }
}

#[async_trait]
impl EventStore for MockEventStore {
    async fn record_cost(&self, event: &CostEvent) -> Result<()> {
        self.events.write().await.push(event.clone());
        Ok(())
    }

    async fn daily_costs(&self, days: u32) -> Result<Vec<f64>> {
        // 가짜 일별 비용 데이터
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
// MockEventBus — 가짜 이벤트 발행기
// ============================================================

pub fn mock_event_bus() -> EventSender {
    let (tx, _) = new_event_bus();
    let sender = tx.clone();

    tokio::spawn(async move {
        let mut seq = 0u64;
        loop {
            let event = GatewayEvent {
                timestamp_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
                event_type: if seq % 3 == 0 {
                    GatewayEventType::CacheHit {
                        cache_tier: "exact".to_string(),
                        model: "gpt-4o".to_string(),
                    }
                } else {
                    GatewayEventType::RequestCompleted {
                        provider_id: "mock-openai".to_string(),
                        model: "gpt-4o".to_string(),
                        latency_ms: 150,
                        tokens: TokenUsage {
                            prompt_tokens: 50,
                            completion_tokens: 100,
                            total_tokens: 150,
                        },
                        cached: false,
                    }
                },
            };
            if sender.send(event).is_err() {
                break;
            }
            seq += 1;
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    tx
}

// ============================================================
// 사용 예시
// ============================================================
//
// #[tokio::test]
// async fn test_with_mock_provider() {
//     let provider = MockProvider::openai();
//     let request = UnifiedRequest { /* ... */ };
//     let response = provider.complete(&request).await.unwrap();
//     assert!(!response.content.is_empty());
// }
//
// #[tokio::test]
// async fn test_with_mock_store() {
//     let store = MockEventStore::new();
//     store.ping().await.unwrap();
// }
