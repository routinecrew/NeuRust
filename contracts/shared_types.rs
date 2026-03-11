// ============================================================
// NeuRust 공유 계약 (Shared Contracts)
// ============================================================
// 모든 에이전트는 이 파일의 타입과 trait을 기준으로 개발한다.
// 이 파일을 수정하려면 반드시 모든 에이전트에게 알려야 한다.
// ============================================================

// ----- 의존성 -----
// tokio = { version = "1", features = ["full"] }
// serde = { version = "1", features = ["derive"] }
// async-trait = "0.1"
// bytes = "1"

use std::collections::HashMap;
use std::fmt;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, oneshot};

// ============================================================
// 1. UnifiedRequest — 프로바이더 독립 요청 표현
// ============================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnifiedRequest {
    /// 대상 모델 (e.g., "gpt-4o", "claude-sonnet-4-20250514")
    pub model: ModelSpec,
    /// 대화 메시지 목록
    pub messages: Vec<Message>,
    /// 생성 온도 (0.0 ~ 2.0)
    pub temperature: Option<f64>,
    /// 최대 출력 토큰
    pub max_tokens: Option<u32>,
    /// 스트리밍 여부
    pub stream: bool,
    /// 요청 컨텍스트 (라우팅/캐싱에 활용)
    pub context: RequestContext,
    /// 원본 프로바이더 고유 파라미터 (passthrough)
    pub extra_params: HashMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelSpec {
    /// 사용자가 지정한 모델 이름
    pub model_name: String,
    /// 프로바이더 ID (라우터가 결정)
    pub provider_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RequestContext {
    /// 요청 유형: interactive | batch
    pub request_type: Option<String>,
    /// 남은 예산 비율 (0.0 ~ 1.0)
    pub budget_remaining_ratio: f64,
    /// 클라이언트 식별자
    pub client_id: Option<String>,
    /// API 키 (인증 후 설정)
    pub api_key: Option<String>,
}

impl UnifiedRequest {
    /// 시스템 메시지 텍스트 추출
    pub fn system_message_text(&self) -> String {
        self.messages
            .iter()
            .filter(|m| m.role == Role::System)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 마지막 사용자 메시지 추출
    pub fn last_user_message(&self) -> String {
        self.messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.clone())
            .unwrap_or_default()
    }
}

// ============================================================
// 2. UnifiedResponse — 프로바이더 독립 응답 표현
// ============================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnifiedResponse {
    /// 응답 텍스트
    pub content: String,
    /// 사용된 모델
    pub model: String,
    /// 토큰 사용량
    pub usage: TokenUsage,
    /// 프로바이더 ID
    pub provider_id: String,
    /// 응답 지연 (밀리초)
    pub latency_ms: u64,
    /// 프로바이더 원본 응답 ID
    pub upstream_id: Option<String>,
}

impl UnifiedResponse {
    /// 전체 텍스트 반환
    pub fn full_text(&self) -> &str {
        &self.content
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

// ============================================================
// 3. StreamChunk — 스트리밍 응답 청크
// ============================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamChunk {
    /// 델타 텍스트
    pub delta: String,
    /// 완료 여부
    pub finished: bool,
    /// 누적 토큰 수 (마지막 청크에만 포함)
    pub usage: Option<TokenUsage>,
}

impl StreamChunk {
    pub fn delta_text(&self) -> &str {
        &self.delta
    }

    pub fn token_count(&self) -> usize {
        // 근사치: 4자 ≈ 1토큰
        (self.delta.len() / 4).max(1)
    }
}

// ============================================================
// 4. PipelineResult — 비스트리밍/스트리밍 응답 구분
// ============================================================

pub enum PipelineResult {
    Complete(UnifiedResponse),
    Stream {
        chunks: Pin<Box<dyn futures_core::Stream<Item = anyhow::Result<StreamChunk>> + Send>>,
        on_complete: Option<oneshot::Sender<UnifiedResponse>>,
    },
}

// ============================================================
// 5. Provider trait — LLM 프로바이더 인터페이스
// ============================================================

#[async_trait]
pub trait Provider: Send + Sync {
    /// 프로바이더 이름 (e.g., "openai", "anthropic")
    fn name(&self) -> &str;

    /// 지원 모델 목록
    fn supported_models(&self) -> Vec<String>;

    /// 비스트리밍 추론
    async fn complete(&self, request: &UnifiedRequest) -> anyhow::Result<UnifiedResponse>;

    /// 스트리밍 추론
    async fn complete_stream(
        &self,
        request: &UnifiedRequest,
    ) -> anyhow::Result<Pin<Box<dyn futures_core::Stream<Item = anyhow::Result<StreamChunk>> + Send>>>;

    /// 헬스 체크
    async fn health_check(&self) -> ProviderHealth;
}

#[derive(Clone, Debug)]
pub struct ProviderHealth {
    pub status: HealthStatus,
    pub latency_ms: u64,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Down,
}

impl fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Healthy => write!(f, "healthy"),
            Self::Degraded => write!(f, "degraded"),
            Self::Down => write!(f, "down"),
        }
    }
}

// ============================================================
// 6. Pipeline trait — 미들웨어 파이프라인 인터페이스
// ============================================================

#[async_trait]
pub trait PipelineLayer: Send + Sync {
    /// 요청 전처리 (파이프라인 진입 시)
    async fn on_request(&self, request: &mut UnifiedRequest) -> anyhow::Result<()> {
        let _ = request;
        Ok(())
    }

    /// 응답 후처리 (파이프라인 탈출 시)
    async fn on_response(
        &self,
        request: &UnifiedRequest,
        response: &mut UnifiedResponse,
    ) -> anyhow::Result<()> {
        let _ = (request, response);
        Ok(())
    }

    /// 레이어 이름 (로그/메트릭용)
    fn name(&self) -> &str;
}

// ============================================================
// 7. EventStore trait — 관측 데이터 저장소 인터페이스
// ============================================================

#[async_trait]
pub trait EventStore: Send + Sync {
    /// 비용 이벤트 저장
    async fn record_cost(&self, event: &CostEvent) -> anyhow::Result<()>;

    /// 일별 비용 조회 (최근 N일)
    async fn daily_costs(&self, days: u32) -> anyhow::Result<Vec<f64>>;

    /// 전체 비용 이벤트 조회
    async fn all_cost_events(&self) -> anyhow::Result<Vec<CostEvent>>;

    /// 메트릭 조회
    async fn all_metrics(&self) -> anyhow::Result<Vec<MetricEntry>>;

    /// 연결 확인
    async fn ping(&self) -> anyhow::Result<()>;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CostEvent {
    pub timestamp_ms: u64,
    pub provider_id: String,
    pub model: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cost_usd: f64,
    pub latency_ms: u64,
    pub cached: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MetricEntry {
    pub timestamp_ms: u64,
    pub metric_name: String,
    pub value: f64,
    pub labels: HashMap<String, String>,
}

// ============================================================
// 8. Config 타입 — 설정 파일 구조
// ============================================================

#[derive(Clone, Debug, Deserialize)]
pub struct NeuRustConfig {
    pub server: ServerConfig,
    pub providers: Vec<ProviderConfig>,
    pub intelligence: Option<IntelligenceConfig>,
    pub security: Option<SecurityConfig>,
    pub observability: Option<ObservabilityConfig>,
    pub auth: Option<AuthConfig>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServerConfig {
    pub address: String,
    pub graceful_shutdown_sec: Option<u32>,
    pub max_stream_buffer_tokens: Option<usize>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub provider_type: String,
    pub api_key_env: String,
    pub base_url: Option<String>,
    pub models: Vec<String>,
    pub priority: Option<u32>,
    pub max_connections: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct IntelligenceConfig {
    pub tiered_cache: Option<TieredCacheConfig>,
    pub cost_router: Option<CostRouterConfig>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TieredCacheConfig {
    pub exact_cache: Option<ExactCacheConfig>,
    pub semantic_cache: Option<SemanticCacheConfig>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ExactCacheConfig {
    pub enabled: bool,
    pub max_entries: Option<usize>,
    pub ttl_sec: Option<u64>,
    pub warming: Option<CacheWarmingConfig>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CacheWarmingConfig {
    pub enabled: bool,
    pub dump_path: Option<String>,
    pub dump_interval_sec: Option<u64>,
    pub max_warm_entries: Option<usize>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SemanticCacheConfig {
    pub enabled: bool,
    pub similarity_threshold: Option<f64>,
    pub eviction: Option<String>,
    pub multiturn_strategy: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CostRouterConfig {
    pub enabled: bool,
    pub warmup_requests: Option<u64>,
    pub epsilon: Option<f64>,
    pub normalize_mode: Option<String>,
    pub bandit_algorithm: Option<String>,
    pub sliding_window_size: Option<usize>,
    pub context_weights: Option<HashMap<String, ContextWeights>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ContextWeights {
    pub cost: f64,
    pub latency: f64,
    pub quality: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SecurityConfig {
    pub pii_masking: Option<PiiConfig>,
    pub prompt_guard: Option<PromptGuardConfig>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PiiConfig {
    pub enabled: bool,
    pub detection_level: Option<String>,
    pub direction: Option<String>,
    pub min_window_tokens: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PromptGuardConfig {
    pub input_scanning: Option<bool>,
    pub role_boundary: Option<bool>,
    pub output_validation: Option<bool>,
    pub output_overlap_mode: Option<String>,
    pub output_overlap_threshold: Option<f32>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ObservabilityConfig {
    pub store: Option<StoreConfig>,
    pub cost_forecaster: Option<CostForecasterConfig>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StoreConfig {
    pub store_type: Option<String>,
    pub sqlite_path: Option<String>,
    pub postgres_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CostForecasterConfig {
    pub enabled: bool,
    pub confidence_interval: Option<f64>,
    pub forecast_horizon_days: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AuthConfig {
    pub api_keys: Option<Vec<ApiKeyEntry>>,
    pub jwt: Option<JwtConfig>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ApiKeyEntry {
    pub key: String,
    pub name: String,
    pub rate_limit: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct JwtConfig {
    pub secret_env: String,
    pub issuer: Option<String>,
}

// ============================================================
// 9. 이벤트 버스 — 시스템 전체 이벤트 전파
// ============================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GatewayEvent {
    pub timestamp_ms: u64,
    pub event_type: GatewayEventType,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GatewayEventType {
    RequestCompleted {
        provider_id: String,
        model: String,
        latency_ms: u64,
        tokens: TokenUsage,
        cached: bool,
    },
    ProviderHealthChanged {
        provider_id: String,
        status: HealthStatus,
    },
    CacheHit {
        cache_tier: String,
        model: String,
    },
    SecurityAlert {
        alert_type: String,
        detail: String,
    },
    ConfigReloaded,
}

pub type EventSender = broadcast::Sender<GatewayEvent>;
pub type EventReceiver = broadcast::Receiver<GatewayEvent>;

/// 이벤트 버스 생성 (기본 버퍼: 1024)
pub fn new_event_bus() -> (EventSender, EventReceiver) {
    broadcast::channel(1024)
}
