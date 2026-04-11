use std::pin::Pin;
use std::time::Instant;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::contracts::*;

/// OpenAI API 프로바이더
pub struct OpenAiProvider {
    id: String,
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    models: Vec<String>,
}

impl OpenAiProvider {
    /// 새 OpenAI 프로바이더 생성
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
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com".to_string()),
            models,
        }
    }

    fn build_request_body(&self, request: &UnifiedRequest) -> OpenAiRequest {
        let messages: Vec<OpenAiMessage> = request
            .messages
            .iter()
            .map(|m| OpenAiMessage {
                role: match m.role {
                    Role::System => "system".to_string(),
                    Role::User => "user".to_string(),
                    Role::Assistant => "assistant".to_string(),
                },
                content: m.content.clone(),
            })
            .collect();

        OpenAiRequest {
            model: request.model.model_name.clone(),
            messages,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: request.stream,
        }
    }
}

#[async_trait]
impl Provider for OpenAiProvider {
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
            .post(format!("{}/v1/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .context("OpenAI request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI API error ({}): {}", status, text);
        }

        let api_resp: OpenAiResponse = resp.json().await.context("Failed to parse OpenAI response")?;
        let latency_ms = start.elapsed().as_millis() as u64;

        let choice = api_resp
            .choices
            .first()
            .context("No choices in OpenAI response")?;

        Ok(UnifiedResponse {
            content: choice.message.content.clone().unwrap_or_default(),
            model: api_resp.model,
            usage: TokenUsage {
                prompt_tokens: api_resp.usage.prompt_tokens,
                completion_tokens: api_resp.usage.completion_tokens,
                total_tokens: api_resp.usage.total_tokens,
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
            .post(format!("{}/v1/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .context("OpenAI stream request failed")?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI API error: {}", text);
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
            loop {
                line_buf.clear();
                match reader.read_line(&mut line_buf).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let line = line_buf.trim();
                        if line.is_empty() || line.starts_with(':') {
                            continue;
                        }
                        if let Some(data) = line.strip_prefix("data: ") {
                            if data == "[DONE]" {
                                yield Ok(StreamChunk {
                                    delta: String::new(),
                                    finished: true,
                                    usage: None,
                                });
                                break;
                            }
                            match serde_json::from_str::<OpenAiStreamChunk>(data) {
                                Ok(chunk) => {
                                    let delta = chunk
                                        .choices
                                        .first()
                                        .and_then(|c| c.delta.content.clone())
                                        .unwrap_or_default();
                                    let finished = chunk
                                        .choices
                                        .first()
                                        .and_then(|c| c.finish_reason.as_ref())
                                        .is_some();
                                    yield Ok(StreamChunk {
                                        delta,
                                        finished,
                                        usage: None,
                                    });
                                }
                                Err(e) => {
                                    tracing::debug!("Skip unparseable SSE chunk: {}", e);
                                }
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
        let start = Instant::now();
        match self
            .client
            .get(format!("{}/v1/models", self.base_url))
            .bearer_auth(&self.api_key)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => ProviderHealth {
                status: HealthStatus::Healthy,
                latency_ms: start.elapsed().as_millis() as u64,
                error: None,
            },
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

// ---- OpenAI API Types ----

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    stream: bool,
}

#[derive(Serialize, Deserialize)]
struct OpenAiMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    id: String,
    model: String,
    choices: Vec<OpenAiChoice>,
    usage: OpenAiUsage,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiChoiceMessage,
}

#[derive(Deserialize)]
struct OpenAiChoiceMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[derive(Deserialize)]
struct OpenAiStreamChunk {
    choices: Vec<OpenAiStreamChoice>,
}

#[derive(Deserialize)]
struct OpenAiStreamChoice {
    delta: OpenAiDelta,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiDelta {
    content: Option<String>,
}
