use std::pin::Pin;
use std::time::Instant;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::contracts::*;

/// Anthropic API 프로바이더
pub struct AnthropicProvider {
    id: String,
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    models: Vec<String>,
}

impl AnthropicProvider {
    /// 새 Anthropic 프로바이더 생성
    pub fn new(
        id: &str,
        api_key: String,
        base_url: Option<String>,
        models: Vec<String>,
    ) -> Self {
        Self {
            id: id.to_string(),
            client: reqwest::Client::new(),
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.anthropic.com".to_string()),
            models,
        }
    }

    fn build_request_body(&self, request: &UnifiedRequest) -> AnthropicRequest {
        let system = request.system_message_text();
        let messages: Vec<AnthropicMessage> = request
            .messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| AnthropicMessage {
                role: match m.role {
                    Role::User => "user".to_string(),
                    Role::Assistant => "assistant".to_string(),
                    Role::System => unreachable!(),
                },
                content: m.content.clone(),
            })
            .collect();

        AnthropicRequest {
            model: request.model.model_name.clone(),
            system: if system.is_empty() {
                None
            } else {
                Some(system)
            },
            messages,
            max_tokens: request.max_tokens.unwrap_or(4096),
            temperature: request.temperature,
            stream: request.stream,
        }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn name(&self) -> &str {
        &self.id
    }

    fn supported_models(&self) -> Vec<String> {
        self.models.clone()
    }

    async fn complete(&self, request: &UnifiedRequest) -> Result<UnifiedResponse> {
        let start = Instant::now();
        let body = self.build_request_body(request);

        let resp = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Anthropic request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API error ({}): {}", status, text);
        }

        let api_resp: AnthropicResponse =
            resp.json().await.context("Failed to parse Anthropic response")?;
        let latency_ms = start.elapsed().as_millis() as u64;

        let content = api_resp
            .content
            .iter()
            .filter_map(|block| {
                if block.content_type == "text" {
                    block.text.clone()
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");

        Ok(UnifiedResponse {
            content,
            model: api_resp.model,
            usage: TokenUsage {
                prompt_tokens: api_resp.usage.input_tokens,
                completion_tokens: api_resp.usage.output_tokens,
                total_tokens: api_resp.usage.input_tokens + api_resp.usage.output_tokens,
            },
            provider_id: self.id.clone(),
            latency_ms,
            upstream_id: Some(api_resp.id),
        })
    }

    async fn complete_stream(
        &self,
        request: &UnifiedRequest,
    ) -> Result<Pin<Box<dyn futures_core::Stream<Item = Result<StreamChunk>> + Send>>> {
        let mut body = self.build_request_body(request);
        body.stream = true;

        let resp = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Anthropic stream request failed")?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API error: {}", text);
        }

        let stream = async_stream::stream! {
            use tokio::io::AsyncBufReadExt;

            let mut reader = tokio::io::BufReader::new(
                tokio_util::io::StreamReader::new(
                    tokio_stream::StreamExt::map(resp.bytes_stream(), |r| {
                        r.map_err(std::io::Error::other)
                    })
                )
            );

            let mut line_buf = String::new();
            let mut current_event = String::new();

            loop {
                line_buf.clear();
                match reader.read_line(&mut line_buf).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let line = line_buf.trim();
                        if line.is_empty() {
                            current_event.clear();
                            continue;
                        }
                        if let Some(event) = line.strip_prefix("event: ") {
                            current_event = event.to_string();
                            continue;
                        }
                        if let Some(data) = line.strip_prefix("data: ") {
                            match current_event.as_str() {
                                "content_block_delta" => {
                                    if let Ok(delta) = serde_json::from_str::<AnthropicStreamDelta>(data) {
                                        if let Some(d) = delta.delta {
                                            yield Ok(StreamChunk {
                                                delta: d.text.unwrap_or_default(),
                                                finished: false,
                                                usage: None,
                                            });
                                        }
                                    }
                                }
                                "message_delta" => {
                                    if let Ok(msg_delta) = serde_json::from_str::<AnthropicMessageDelta>(data) {
                                        yield Ok(StreamChunk {
                                            delta: String::new(),
                                            finished: true,
                                            usage: msg_delta.usage.map(|u| TokenUsage {
                                                prompt_tokens: 0,
                                                completion_tokens: u.output_tokens,
                                                total_tokens: u.output_tokens,
                                            }),
                                        });
                                    }
                                }
                                "message_stop" => {
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(e) => {
                        yield Err(anyhow::anyhow!("Stream read error: {}", e));
                        break;
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }

    async fn health_check(&self) -> ProviderHealth {
        // Anthropic doesn't expose /models; send a minimal request to verify connectivity.
        let start = Instant::now();
        match self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&serde_json::json!({
                "model": self.models.first().cloned().unwrap_or_else(|| "claude-sonnet-4-20250514".to_string()),
                "max_tokens": 1,
                "messages": [{"role": "user", "content": "hi"}]
            }))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 400 => {
                // 400 is acceptable — means API is reachable and auth works
                ProviderHealth {
                    status: HealthStatus::Healthy,
                    latency_ms: start.elapsed().as_millis() as u64,
                    error: None,
                }
            }
            Ok(resp) if resp.status().as_u16() == 401 || resp.status().as_u16() == 403 => {
                ProviderHealth {
                    status: HealthStatus::Degraded,
                    latency_ms: start.elapsed().as_millis() as u64,
                    error: Some(format!("Auth error: HTTP {}", resp.status())),
                }
            }
            Ok(resp) => ProviderHealth {
                status: HealthStatus::Degraded,
                latency_ms: start.elapsed().as_millis() as u64,
                error: Some(format!("HTTP {}", resp.status())),
            },
            Err(e) => ProviderHealth {
                status: HealthStatus::Down,
                latency_ms: start.elapsed().as_millis() as u64,
                error: Some(e.to_string()),
            },
        }
    }
}

// ---- Anthropic API Types ----

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<AnthropicMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    stream: bool,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    id: String,
    model: String,
    content: Vec<ContentBlock>,
    usage: AnthropicUsage,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Deserialize)]
struct AnthropicStreamDelta {
    delta: Option<AnthropicTextDelta>,
}

#[derive(Deserialize)]
struct AnthropicTextDelta {
    text: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicMessageDelta {
    usage: Option<AnthropicDeltaUsage>,
}

#[derive(Deserialize)]
struct AnthropicDeltaUsage {
    output_tokens: u32,
}
