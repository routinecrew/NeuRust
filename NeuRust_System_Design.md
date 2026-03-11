# NeuRust — 시스템 설계서 (v3.0 — 2차 전문가 검증 반영)

> The Intelligent Rust Gateway for Nano-second AI Routing
>
> **개정 이력**
> - v1.0: 초기 설계
> - v2.0: 1차 검증 15개 치명적/중요 개선 반영 (77.7→85.7점)
> - v3.0: 2차 검증 12개 잔여 이슈 전량 반영 (본 문서)
>
> `[v2]` = 1차 검증 반영, `[v3]` = 2차 검증 반영

---

## 1. 경쟁사 소스코드 구조 분석

(v2.0과 동일 — 변경 없음. TensorZero, Helicone, Bifrost, Traceloop Hub 분석)

---

## 2. NeuRust 전체 아키텍처

### 2.1 핵심 설계 원칙

1. **Data Plane / Control Plane 분리**
2. **파이프라인 미들웨어 체인** (Tower)
3. **[v2] Lean 크레이트 전략** — 3개 + **[v3] Cargo feature flags**
4. **[v2] Zero-Config 온보딩** — 최소 3줄 설정

### 2.2 전체 구조

```
neurust (binary)
  └─ Gateway
       ├─ config::Config              ← serde + YAML (Hot Reload)
       ├─ config::Watcher             ← notify crate
       ├─ auth::AuthManager           ← API Key + JWT
       │
       ├─ pipeline::Pipeline          ← ★ Tower 미들웨어 체인
       │    ├─ Layer: RateLimiter     ← GCRA
       │    ├─ Layer: AuthLayer       ← 인증
       │    ├─ Layer: PiiMasker       ← 🔒 [v2] 양방향 PII 마스킹
       │    ├─ Layer: TieredCache     ← 🧠 [v2] Exact→Semantic 2단계
       │    ├─ Layer: CostRouter      ← 🧠 [v2] 정규화 + [v3] safe-normalize
       │    ├─ Layer: ComplexityGate  ← 🧠 복잡도 분류
       │    ├─ Layer: LoadBalancer    ← LB
       │    ├─ Layer: Fallback        ← 재시도
       │    └─ Layer: Observability   ← 메트릭
       │
       ├─ provider::ProviderRegistry  ← [v2] ConnectionPool
       ├─ intelligence::
       │    ├─ TieredCache            ← [v2] + [v3] 턴 수 포함 해시 + 캐시 워밍
       │    ├─ CostAuctioneer         ← [v2] + [v3] safe-normalize + SW-UCB
       │    ├─ BanditRouter           ← [v2] ε-greedy + [v3] Sliding Window
       │    └─ HealthMonitor
       ├─ security::
       │    ├─ PiiDetector            ← [v2] 3단계 + [v3] 동적 윈도우
       │    ├─ PiiMasker              ← [v2] 양방향 + [v3] adaptive window
       │    ├─ PromptGuard            ← [v2] 3방어 + [v3] 비율 기반 임계치
       │    └─ AuditLogger            ← [v2]
       ├─ observability::
       │    ├─ CostTracker + CostForecaster ← [v2] + [v3] 신뢰구간
       │    └─ MetricsCollector + TraceExporter
       ├─ api::ApiServer              ← [v3] health deep/shallow 분리
       └─ store::                     ← [v2] Tiered + [v3] 마이그레이션 도구
```

### 2.3 `[v3 개선]` 디렉토리 구조 — Feature Flags 추가

```
neurust/
├── Cargo.toml
├── crates/
│   ├── neurust-core/                   ← 핵심 (provider + router + config + auth)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── config.rs
│   │       ├── pipeline.rs             ← [v2] Dual-path + [v3] 버퍼 상한
│   │       ├── request.rs              ← [v2] PipelineResult
│   │       ├── error.rs
│   │       ├── provider/
│   │       │   ├── mod.rs              ← Provider trait + ConnectionPool
│   │       │   ├── openai.rs
│   │       │   ├── anthropic.rs
│   │       │   └── openai_compat.rs
│   │       ├── router/
│   │       │   ├── load_balancer.rs
│   │       │   ├── fallback.rs
│   │       │   └── health.rs           ← [v3] deep/shallow 분리
│   │       └── auth/
│   │
│   ├── neurust-gateway/
│   │   └── src/
│   │       ├── main.rs
│   │       ├── server.rs               ← [v2] Graceful Shutdown
│   │       ├── routes/
│   │       │   ├── inference.rs        ← [v3] 스트리밍 버퍼 상한 적용
│   │       │   ├── embeddings.rs
│   │       │   ├── models.rs
│   │       │   ├── health.rs           ← [v3] /health/live + /health/ready
│   │       │   └── admin.rs
│   │       └── middleware/
│   │
│   └── neurust-intel/                  ← [v3] Cargo feature flags 적용
│       ├── Cargo.toml                  ← [v3] features 정의
│       └── src/
│           ├── cache/
│           │   ├── mod.rs              ← TieredCache
│           │   ├── exact_cache.rs      ← [v3] 턴 수 포함 해시
│           │   ├── semantic_cache.rs   ← #[cfg(feature = "cache-semantic")]
│           │   ├── cache_key.rs        ← [v3] 개선된 멀티턴 키
│           │   ├── eviction.rs         ← LFU
│           │   └── warming.rs          ← [v3] 신규 — 캐시 워밍
│           ├── router/
│           │   ├── cost_auctioneer.rs  ← [v3] safe_normalize + SW-UCB
│           │   ├── bandit_router.rs    ← [v3] SlidingWindowUCB
│           │   ├── complexity_gate.rs
│           │   └── price_db.rs
│           ├── security/               ← #[cfg(feature = "security")]
│           │   ├── pii_detector.rs     ← [v3] 동적 윈도우 크기
│           │   ├── pii_masker.rs       ← [v3] adaptive sliding buffer
│           │   ├── context_validator.rs
│           │   ├── prompt_guard.rs     ← [v3] 비율 기반 Output Validation
│           │   └── audit_logger.rs
│           ├── observability/
│           │   ├── cost_tracker.rs
│           │   ├── cost_forecaster.rs  ← [v3] 신뢰구간 포함
│           │   └── budget_manager.rs
│           └── store/
│               ├── mod.rs
│               ├── sqlite_store.rs
│               ├── postgres_store.rs
│               ├── clickhouse_store.rs
│               └── migrator.rs         ← [v3] 신규 — SQLite→PG 마이그레이션
│
├── data/
│   └── model_prices.json
├── benches/
├── tests/e2e/
│   ├── proxy/
│   ├── streaming/
│   ├── cache/
│   ├── routing/
│   └── security/
└── .github/                            ← [v3] 신규
    └── workflows/
        └── ci.yml                      ← [v3] CI/CD 파이프라인
```

### 2.4 `[v3 개선]` Cargo Feature Flags — 선택적 컴파일

`neurust-intel`이 21개 소스 파일로 과대한 문제를 크레이트 분리 없이 해결한다.

```toml
# crates/neurust-intel/Cargo.toml

[package]
name = "neurust-intel"
version = "0.1.0"

[features]
default = ["cache-exact", "observability"]

# 캐싱
cache-exact = []                              # Exact Cache만 (의존성 0, 기본 ON)
cache-semantic = [                            # Semantic Cache (무거운 의존성)
    "dep:fastembed",
    "dep:simsimd",
    "dep:hnsw_rs"
]

# 보안
security = ["dep:aho-corasick"]               # PII + PromptGuard
security-ner = ["security", "dep:ort"]        # + NER 모델 (Level 3)

# 비용 라우터
cost-router = []                              # CostAuctioneer + Bandit

# 관측
observability = []                            # CostTracker + Metrics

# 저장소
store-sqlite = ["dep:rusqlite"]               # 기본
store-postgres = ["dep:sqlx"]                 # 중규모
store-clickhouse = ["dep:clickhouse-rs"]      # 대규모

# 전체
full = [
    "cache-exact", "cache-semantic",
    "security", "cost-router",
    "observability",
    "store-sqlite", "store-postgres", "store-clickhouse"
]

[dependencies]
# 항상 포함
dashmap = "6"
xxhash-rust = "0.8"
tokio = { version = "1", features = ["full"] }

# 선택적
fastembed = { version = "4", optional = true }
simsimd = { version = "0.5", optional = true }
hnsw_rs = { version = "0.3", optional = true }
aho-corasick = { version = "1", optional = true }
ort = { version = "2", optional = true }
rusqlite = { version = "0.31", optional = true }
sqlx = { version = "0.8", features = ["postgres"], optional = true }
clickhouse-rs = { version = "1", optional = true }
```

**효과:**
- `cargo build` (기본): Exact Cache + Observability만 컴파일 → 빌드 30초 이내
- `cargo build --features full`: 전체 컴파일 → 빌드 2~3분
- fastembed, ort 같은 무거운 의존성이 필요한 사용자만 활성화

---

## 3. 핵심 컴포넌트 상세 설계

### 3.1 PipelineResult + `[v3]` 스트리밍 버퍼 상한

```rust
pub enum PipelineResult {
    Complete(UnifiedResponse),
    Stream {
        chunks: Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>,
        on_complete: oneshot::Sender<UnifiedResponse>,
    },
}

/// [v3 개선] 스트리밍 응답 조립기 — 버퍼 상한 포함
pub struct StreamAssembler {
    buffer: Vec<StreamChunk>,
    total_tokens: usize,
    max_buffer_tokens: usize,           // [v3] 기본값: 128K 토큰
    truncated: bool,
}

impl StreamAssembler {
    pub fn new(config: &StreamConfig) -> Self {
        Self {
            buffer: Vec::with_capacity(1024),
            total_tokens: 0,
            max_buffer_tokens: config.max_buffer_tokens.unwrap_or(128_000),
            truncated: false,
        }
    }

    /// 청크를 버퍼에 추가 — 상한 초과 시 조기 종료
    pub fn push(&mut self, chunk: &StreamChunk) {
        self.total_tokens += chunk.token_count();

        if self.total_tokens <= self.max_buffer_tokens {
            self.buffer.push(chunk.clone());
        } else if !self.truncated {
            self.truncated = true;
            tracing::warn!(
                "Stream assembler buffer limit reached ({} tokens). \
                 Cache storage will be skipped for this response.",
                self.max_buffer_tokens
            );
        }
    }

    /// 조립 완료 — 캐시 저장 가능 여부 반환
    pub fn finalize(self) -> Option<UnifiedResponse> {
        if self.truncated {
            return None;  // 버퍼 초과 시 캐시 저장 포기
        }
        Some(self.assemble_response())
    }
}
```

### 3.2 UnifiedRequest / UnifiedResponse

(v2.0과 동일 — RequestContext, CacheTier 포함)

### 3.3 Pipeline

(v2.0과 동일 — Dual-path Tower 미들웨어)

### 3.4 Provider

(v2.0과 동일 — ConnectionPool, TokenPricing 포함)

### 3.5 `[v3 개선]` Health Check — Deep/Shallow 분리

```rust
/// [v3] K8s liveness probe (shallow) — 프로세스 살아있는지만 확인
/// GET /health/live → 200 OK (즉시 응답, <0.01ms)
pub async fn health_live() -> StatusCode {
    StatusCode::OK
}

/// [v3] K8s readiness probe (deep) — 프로바이더 연결까지 확인
/// GET /health/ready → 200 OK 또는 503 Service Unavailable
pub async fn health_ready(
    providers: Arc<ProviderRegistry>,
    store: Arc<dyn EventStore>,
) -> (StatusCode, Json<HealthDetail>) {
    let mut checks = Vec::new();

    // 프로바이더 연결 확인
    for provider in providers.all() {
        let health = provider.health_check().await;
        checks.push(ComponentHealth {
            name: provider.name().to_string(),
            status: health.status,
            latency_ms: health.latency_p50.as_millis() as u64,
        });
    }

    // 저장소 연결 확인
    let store_ok = store.ping().await.is_ok();
    checks.push(ComponentHealth {
        name: "store".into(),
        status: if store_ok { HealthStatus::Healthy } else { HealthStatus::Down },
        latency_ms: 0,
    });

    let all_healthy = checks.iter().all(|c| c.status != HealthStatus::Down);
    let status = if all_healthy { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };

    (status, Json(HealthDetail { components: checks }))
}
```

### 3.6 Config

(v2.0 Zero-Config와 동일 + 아래 추가 항목)

```yaml
# [v3] 추가 설정 항목들

server:
  address: "0.0.0.0:8080"
  graceful_shutdown_sec: 30
  max_stream_buffer_tokens: 128000      # [v3] 스트리밍 조립 버퍼 상한

intelligence:
  tiered_cache:
    exact_cache:
      enabled: true
      warming:                          # [v3] 캐시 워밍
        enabled: true
        dump_path: "./cache_dump.bin"   # 상위 N개 항목 디스크 덤프
        dump_interval_sec: 300          # 5분마다 덤프
        max_warm_entries: 10000         # 시작 시 최대 로딩 수
    semantic_cache:
      enabled: false
      similarity_threshold: 0.92
      eviction: lfu
      multiturn_strategy: last_message_with_turn_count  # [v3] 턴 수 포함

  cost_router:
    enabled: false
    warmup_requests: 100
    epsilon: 0.1
    normalize_mode: safe                # [v3] safe | standard
    bandit_algorithm: sliding_window_ucb  # [v3] ucb1 | sliding_window_ucb
    sliding_window_size: 500            # [v3] 최근 N회 관측만 사용
    context_weights:
      interactive: { cost: 0.2, latency: 0.6, quality: 0.2 }
      batch:       { cost: 0.7, latency: 0.1, quality: 0.2 }

security:
  pii_masking:
    enabled: false
    detection_level: balanced
    direction: bidirectional
    min_window_tokens: auto             # [v3] auto = 가장 긴 PII 패턴 + 20토큰
  prompt_guard:
    input_scanning: true
    role_boundary: true
    output_validation: true
    output_overlap_mode: ratio          # [v3] ratio | absolute
    output_overlap_threshold: 0.3       # [v3] 시스템 프롬프트의 30% 겹치면 누출

observability:
  store:
    type: auto
    migration:                          # [v3] 마이그레이션 설정
      auto_suggest: true                # 한계 도달 시 업그레이드 안내
      tool: bundled                     # bundled | external
  cost_forecaster:
    enabled: true
    confidence_interval: 0.85           # [v3] 85% 신뢰구간
    forecast_horizon_days: 30           # [v3] 예측 기간
```

---

## 4. 🧠 Intelligence Layer

### 4.1 TieredCache + `[v3]` 턴 수 포함 해시 + 캐시 워밍

```rust
pub struct TieredCache {
    exact: ExactCache,
    semantic: Option<SemanticCache>,      // #[cfg(feature = "cache-semantic")]
    warmer: Option<CacheWarmer>,          // [v3]
}

impl ExactCache {
    /// [v3 개선] 멀티턴 Exact Hash — 턴 수 포함으로 대화 기록 충돌 방지
    ///
    /// v2.0 문제: "파이썬으로 함수 만들어줘"가 1턴 대화와 5턴 대화에서
    /// 같은 해시를 생성하여 잘못된 캐시 히트 발생
    ///
    /// v3.0 해결: messages.len()을 해시에 포함
    fn compute_hash(&self, request: &UnifiedRequest) -> u64 {
        let mut hasher = XxHash64::default();
        hasher.write(request.system_message_text().as_bytes());
        hasher.write(request.last_user_message().as_bytes());
        hasher.write(request.model.model_name.as_bytes());
        hasher.write(&request.messages.len().to_le_bytes());  // [v3] 턴 수 포함
        // temperature가 있으면 해시에 포함 (다른 temperature → 다른 응답)
        if let Some(temp) = request.temperature {
            hasher.write(&temp.to_le_bytes());                // [v3] temperature 포함
        }
        hasher.finish()
    }
}

/// [v3 개선] 캐시 워밍 — 서버 재시작 시 cold start 방지
pub struct CacheWarmer {
    dump_path: PathBuf,
    dump_interval: Duration,
    max_warm_entries: usize,
}

impl CacheWarmer {
    /// 서버 시작 시 디스크에서 캐시 로딩
    pub async fn warm(&self, cache: &ExactCache) -> Result<usize> {
        if !self.dump_path.exists() {
            return Ok(0);
        }

        let data: Vec<CacheDumpEntry> = bincode::deserialize(
            &tokio::fs::read(&self.dump_path).await?
        )?;

        let loaded = data.into_iter()
            .take(self.max_warm_entries)
            .filter(|e| !e.is_expired())
            .map(|e| cache.insert_warm(e.hash, e.response, e.hit_count))
            .count();

        tracing::info!("Cache warmed with {} entries from {}", loaded, self.dump_path.display());
        Ok(loaded)
    }

    /// 주기적 디스크 덤프 (백그라운드 태스크)
    pub async fn dump_loop(&self, cache: Arc<ExactCache>) {
        let mut interval = tokio::time::interval(self.dump_interval);
        loop {
            interval.tick().await;
            let top_entries = cache.top_by_hit_count(self.max_warm_entries);
            let bytes = bincode::serialize(&top_entries).unwrap_or_default();
            let _ = tokio::fs::write(&self.dump_path, bytes).await;
        }
    }
}
```

### 4.2 CostAuctioneer + `[v3]` safe_normalize + Sliding Window UCB

```rust
impl CostAuctioneer {
    pub async fn select_best(&self, request: &UnifiedRequest) -> Result<ModelSpec> {
        // [v2] Cold Start 체크
        let bandit = self.bandit.read().await;
        if bandit.total_requests() < self.config.warmup_requests {
            return self.select_by_priority();
        }

        let raw_scores = self.collect_raw_scores(request).await;

        // [v3 개선] safe_normalize — 후보 1~2개일 때 안전 처리
        let costs: Vec<f64> = raw_scores.iter().map(|s| s.cost).collect();
        let latencies: Vec<f64> = raw_scores.iter().map(|s| s.latency_ms).collect();

        // [v2] 컨텍스트 기반 동적 가중치
        let weights = self.get_context_weights(&request.context);
        let budget_factor = if request.context.budget_remaining_ratio < 0.2 { 1.5 } else { 1.0 };

        let mut scored: Vec<(String, f64)> = raw_scores.iter()
            .map(|s| {
                let cost_norm = safe_normalize(s.cost, &costs);       // [v3]
                let lat_norm = safe_normalize(s.latency_ms, &latencies); // [v3]
                let qual_norm = s.quality;

                let combined = cost_norm * weights.cost * budget_factor
                    + lat_norm * weights.latency
                    + (1.0 - qual_norm) * weights.quality;

                (s.model_id.clone(), combined)
            })
            .collect();

        // [v2] ε-greedy 탐색
        if rand::random::<f64>() < self.config.epsilon {
            return self.random_choice(&scored);
        }

        scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        self.to_model_spec(&scored[0].0)
    }
}

/// [v3 개선] 안전한 정규화 — 후보 1개 또는 min==max 시 안전 처리
///
/// v2.0 문제: 후보 1개일 때 (min - min) / (max - min) = 0/0 → NaN
/// v2.0 문제: 후보 2개, 같은 값일 때도 0/0
///
/// v3.0 해결:
///   - 후보 1개: 0.5 반환 (중립)
///   - min == max: 모든 값 0.5 반환 (동점 처리)
///   - 정상: 0.0 ~ 1.0 min-max 정규화
fn safe_normalize(value: f64, all_values: &[f64]) -> f64 {
    if all_values.len() <= 1 {
        return 0.5;  // 후보 1개 → 중립
    }

    let min = all_values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = all_values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;

    if range < f64::EPSILON {
        return 0.5;  // min == max → 동점
    }

    (value - min) / range  // 0.0 ~ 1.0
}

/// [v3 개선] Sliding Window UCB — 최근 N회 관측만 사용
///
/// v2.0 UCB1: 모든 과거 데이터를 동일 가중치로 사용
/// → 프로바이더 성능이 변동해도 과거 데이터가 희석
///
/// v3.0 SW-UCB: 최근 window_size 회만 사용
/// → 프로바이더가 느려지면 빠르게 점수에 반영
pub struct SlidingWindowUCB {
    window_size: usize,                   // 기본 500
    arms: HashMap<String, VecDeque<Observation>>,
}

struct Observation {
    reward: f64,                          // 0.0(실패/느림) ~ 1.0(성공/빠름)
    timestamp: Instant,
}

impl SlidingWindowUCB {
    pub fn get_score(&self, model_id: &str) -> f64 {
        let arm = match self.arms.get(model_id) {
            Some(a) if !a.is_empty() => a,
            _ => return 0.5,  // 데이터 없음 → 중립
        };

        let n = arm.len() as f64;
        let total_n: f64 = self.arms.values().map(|a| a.len() as f64).sum();

        let mean_reward = arm.iter().map(|o| o.reward).sum::<f64>() / n;
        let exploration = (2.0 * total_n.ln() / n).sqrt();

        mean_reward + exploration  // UCB 점수
    }

    pub fn update(&mut self, model_id: &str, reward: f64) {
        let arm = self.arms.entry(model_id.to_string()).or_default();
        arm.push_back(Observation { reward, timestamp: Instant::now() });

        // 윈도우 크기 초과 시 오래된 관측 제거
        while arm.len() > self.window_size {
            arm.pop_front();
        }
    }
}
```

---

## 5. 🔒 Security Layer

### 5.1 PiiDetector + `[v3]` 동적 슬라이딩 윈도우

```rust
pub struct PiiMasker {
    detector: Arc<PiiDetector>,
    direction: Direction,
    audit_logger: Arc<AuditLogger>,
    min_window_tokens: usize,             // [v3] 동적 윈도우 크기
}

impl PiiMasker {
    pub fn new(detector: Arc<PiiDetector>, config: &PiiConfig) -> Self {
        // [v3 개선] 슬라이딩 윈도우 최소 크기를 동적 계산
        //
        // v2.0 문제: 고정 50토큰 윈도우에서 PII가 청크 경계에 걸치면 미탐지
        // 예: chunk1="주민번호 901234-" + chunk2="1234567입니다"
        //
        // v3.0 해결: 가장 긴 PII 패턴 + 마진으로 동적 계산
        let min_window = if config.min_window_tokens == "auto" {
            let max_pattern_tokens = detector.max_pattern_length_tokens(); // 예: 주민번호 14자리 ≈ 7토큰
            max_pattern_tokens + 20  // 마진 20토큰
        } else {
            config.min_window_tokens.parse().unwrap_or(50)
        };

        Self {
            detector,
            direction: config.direction,
            audit_logger: Arc::new(AuditLogger::new()),
            min_window_tokens: min_window,
        }
    }

    /// [v3 개선] 적응형 슬라이딩 버퍼 — 동적 윈도우 크기 적용
    pub fn mask_stream_chunk(
        &self,
        buffer: &mut SlidingBuffer,
        chunk: &StreamChunk,
    ) -> StreamChunk {
        buffer.push(&chunk.delta_text());

        // [v3] 동적 윈도우: 최소 min_window_tokens, 최대 2x
        let window_text = buffer.last_n_tokens(self.min_window_tokens);
        let matches = self.detector.detect(&window_text);

        if !matches.is_empty() {
            self.audit_logger.log_stream_masking(&chunk, &matches);
            self.apply_mask_to_chunk(chunk, &matches, buffer)
        } else {
            chunk.clone()
        }
    }
}
```

### 5.2 PromptGuard + `[v3]` 비율 기반 Output Validation

```rust
impl PromptGuard {
    pub fn scan(&self, request: &UnifiedRequest) -> Result<()> {
        // 방어 1: Input Scanning (v2 — 변경 없음)
        // 방어 2: Role Boundary (v2 — 변경 없음)
        Ok(())
    }

    /// [v3 개선] Output Validation — 비율 기반 겹침 탐지
    ///
    /// v2.0 문제: "20단어 이상 겹치면 누출" 기준이 짧은 시스템 프롬프트에서 무의미
    /// 예: "너는 도움이 되는 AI야" (8단어) → 20단어 기준 도달 불가능
    ///
    /// v3.0 해결: 시스템 프롬프트 대비 비율로 판단
    pub fn validate_output(
        &self,
        request: &UnifiedRequest,
        response: &UnifiedResponse,
    ) -> OutputValidationResult {
        if !self.config.output_validation { return OutputValidationResult::Pass; }

        let system_prompt = request.system_message_text();
        if system_prompt.is_empty() { return OutputValidationResult::Pass; }

        let response_text = response.full_text();
        let system_words: Vec<&str> = system_prompt.split_whitespace().collect();
        let response_words: Vec<&str> = response_text.split_whitespace().collect();

        // [v3] 시스템 프롬프트의 연속 N-gram이 응답에 포함된 비율 계산
        let overlap_ratio = compute_ngram_overlap(&system_words, &response_words, 4);
        // 4-gram(4단어 연속)으로 의미 있는 구문 겹침을 측정

        let threshold = match self.config.output_overlap_mode {
            OverlapMode::Ratio => self.config.output_overlap_threshold,   // [v3] 기본 0.3
            OverlapMode::Absolute => {
                // 절대 단어 수를 비율로 변환
                self.config.output_overlap_threshold / system_words.len() as f32
            }
        };

        if overlap_ratio > threshold {
            OutputValidationResult::Leak {
                overlap_ratio,
                threshold,
            }
        } else {
            OutputValidationResult::Pass
        }
    }
}

pub enum OutputValidationResult {
    Pass,
    Leak { overlap_ratio: f32, threshold: f32 },
}

/// [v3] N-gram 겹침 비율 계산
fn compute_ngram_overlap(source: &[&str], target: &[&str], n: usize) -> f32 {
    if source.len() < n { return 0.0; }

    let source_ngrams: HashSet<Vec<&str>> = source
        .windows(n)
        .map(|w| w.to_vec())
        .collect();

    let target_ngrams: HashSet<Vec<&str>> = target
        .windows(n)
        .map(|w| w.to_vec())
        .collect();

    let overlap = source_ngrams.intersection(&target_ngrams).count();
    overlap as f32 / source_ngrams.len() as f32
}
```

---

## 6. 관측 + `[v3]` CostForecaster 신뢰구간 + 마이그레이션

### 6.1 `[v3 개선]` CostForecaster — 신뢰구간 포함

```rust
pub struct CostForecast {
    pub estimated_monthly_cost: f64,
    pub lower_bound: f64,                 // [v3] 신뢰구간 하한
    pub upper_bound: f64,                 // [v3] 신뢰구간 상한
    pub confidence_level: f64,            // [v3] 0.85 = 85%
    pub trend: CostTrend,                 // Rising | Stable | Falling
    pub forecast_date: String,            // 예측 기준일
    pub data_points: usize,               // 예측에 사용된 데이터 수
}

pub struct CostForecaster {
    config: ForecastConfig,
}

impl CostForecaster {
    /// [v3 개선] 월말 비용 예측 + 신뢰구간
    pub async fn forecast(&self, store: &dyn EventStore) -> Result<CostForecast> {
        // 최근 N일간 일별 비용 데이터 수집
        let daily_costs = store.daily_costs(self.config.forecast_horizon_days).await?;

        if daily_costs.len() < 3 {
            return Err(Error::InsufficientData("At least 3 days of data required"));
        }

        // 선형 회귀로 추세 추정
        let (slope, intercept) = linear_regression(&daily_costs);
        let remaining_days = days_until_month_end();
        let current_total: f64 = daily_costs.iter().sum();
        let projected_remaining = (0..remaining_days)
            .map(|d| slope * (daily_costs.len() + d) as f64 + intercept)
            .sum::<f64>();

        let estimated = current_total + projected_remaining;

        // [v3] 잔차의 표준편차로 신뢰구간 계산
        let residuals: Vec<f64> = daily_costs.iter().enumerate()
            .map(|(i, &actual)| actual - (slope * i as f64 + intercept))
            .collect();
        let std_dev = standard_deviation(&residuals);

        // z-score: 85% 신뢰구간 → 1.44
        let z = z_score(self.config.confidence_interval);
        let margin = z * std_dev * (remaining_days as f64).sqrt();

        Ok(CostForecast {
            estimated_monthly_cost: estimated,
            lower_bound: (estimated - margin).max(current_total), // 현재까지 쓴 것보다 낮을 수 없음
            upper_bound: estimated + margin,
            confidence_level: self.config.confidence_interval,
            trend: if slope > 0.01 { CostTrend::Rising }
                   else if slope < -0.01 { CostTrend::Falling }
                   else { CostTrend::Stable },
            forecast_date: today_str(),
            data_points: daily_costs.len(),
        })
    }
}
```

### 6.2 `[v3 개선]` Store Migrator — SQLite → PostgreSQL

```rust
/// [v3] 저장소 마이그레이션 도구
pub struct StoreMigrator;

impl StoreMigrator {
    /// SQLite → PostgreSQL 데이터 마이그레이션
    pub async fn migrate_sqlite_to_postgres(
        sqlite_path: &str,
        postgres_url: &str,
    ) -> Result<MigrationReport> {
        let sqlite = SqliteStore::open(sqlite_path)?;
        let pg = PostgresStore::connect(postgres_url).await?;

        // 스키마 생성
        pg.ensure_schema().await?;

        // 배치 단위로 데이터 전송
        let mut total_rows = 0;
        let batch_size = 1000;

        // 비용 이벤트 마이그레이션
        let cost_events = sqlite.all_cost_events().await?;
        for chunk in cost_events.chunks(batch_size) {
            pg.bulk_insert_cost_events(chunk).await?;
            total_rows += chunk.len();
        }

        // 캐시 메트릭 마이그레이션
        let metrics = sqlite.all_metrics().await?;
        for chunk in metrics.chunks(batch_size) {
            pg.bulk_insert_metrics(chunk).await?;
            total_rows += chunk.len();
        }

        Ok(MigrationReport {
            source: sqlite_path.to_string(),
            destination: postgres_url.to_string(),
            total_rows,
            duration: start.elapsed(),
        })
    }
}

// CLI에서 실행: neurust migrate --from sqlite:./data.db --to postgres://...
```

---

## 7. `[v3 개선]` CI/CD 파이프라인 설계

```yaml
# .github/workflows/ci.yml
name: NeuRust CI

on:
  push:
    branches: [main]
  pull_request:

env:
  CARGO_TERM_COLOR: always
  RUSTFLAGS: "-D warnings"

jobs:
  # Stage 1: 빠른 검증 (1~2분)
  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - run: cargo fmt --all -- --check
      - run: cargo clippy --all-targets --all-features -- -D warnings

  # Stage 2: 단위 테스트 (2~3분)
  test-unit:
    runs-on: ubuntu-latest
    needs: lint
    strategy:
      matrix:
        features:
          - "default"                      # Exact Cache + Observability
          - "cache-semantic,security"       # + Semantic + PII
          - "full"                          # 전체
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace --features ${{ matrix.features }}

  # Stage 3: 벤치마크 (3~5분, main 브랜치만)
  benchmark:
    if: github.ref == 'refs/heads/main'
    runs-on: ubuntu-latest
    needs: test-unit
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo bench --bench pipeline_bench -- --output-format bencher
      - run: cargo bench --bench exact_cache_bench -- --output-format bencher

  # Stage 4: E2E 테스트 (5~10분, Docker 필요)
  test-e2e:
    runs-on: ubuntu-latest
    needs: test-unit
    services:
      # 테스트용 mock LLM 서버
      mock-provider:
        image: neurust/mock-provider:latest
        ports: ["8888:8888"]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --test e2e -- --test-threads=1
        env:
          MOCK_PROVIDER_URL: "http://localhost:8888"

  # Stage 5: Docker 이미지 빌드 (main 머지 시)
  docker:
    if: github.ref == 'refs/heads/main'
    runs-on: ubuntu-latest
    needs: [test-unit, test-e2e]
    steps:
      - uses: actions/checkout@v4
      - uses: docker/build-push-action@v5
        with:
          push: true
          tags: ghcr.io/neurust/neurust:latest
```

---

## 8. 데이터 흐름

(v2.0과 동일 — 비스트리밍, 스트리밍, 2단계 캐시 히트 경로)

**[v3] 추가: 스트리밍 조립 버퍼 초과 시**

```
Client → ①~⑨ → Provider SSE 시작
← 토큰 passthrough (정상 전달)
← 백그라운드 조립기: 128K 토큰 상한 도달
← StreamAssembler: truncated=true → 캐시 저장 포기 (warn 로그)
← 응답은 정상 완료, 캐시만 미저장
```

---

## 9. 구현 로드맵

### Phase 1: Core Proxy (2개월)
- `neurust-core`: UnifiedRequest + PipelineResult (Dual-path)
- `neurust-gateway`: axum + SSE + Graceful Shutdown + [v3] health deep/shallow
- OpenAI/Anthropic + ConnectionPool
- Fallback + Retry + Exact Cache (해시 기반)
- [v3] Exact Hash에 턴 수 + temperature 포함
- [v3] CI/CD 파이프라인 설정

### Phase 2: Intelligence (2개월)
- TieredCache (Exact + Semantic) + [v3] 캐시 워밍
- CostAuctioneer [v3] safe_normalize + Sliding Window UCB
- [v3] 스트리밍 조립 버퍼 상한
- 커뮤니티 가격 DB

### Phase 3: Security (1.5개월)
- PiiDetector 3단계 + [v3] 동적 슬라이딩 윈도우
- PromptGuard [v3] 비율 기반 Output Validation
- AuditLogger

### Phase 4: Observability (1.5개월)
- Tiered Storage + [v3] SQLite→PG Migrator
- CostForecaster [v3] 신뢰구간 포함

### Phase 5: 확장 (2개월)
- 추가 프로바이더 + SDK
- 멀티 인스턴스 Redis shared state

---

## 10. 의사결정 기록

| 결정 사항 | 선택 | 이유 | 출처 |
|-----------|------|------|------|
| **[v3]** Feature Flags | Cargo features | 21개 파일 크레이트의 빌드 시간 관리. fastembed/ort 격리 | 2차 검증 |
| **[v3]** Exact Hash 키 | 턴 수 + temperature 포함 | 멀티턴 대화 충돌 방지 | 2차 검증 |
| **[v3]** 캐시 워밍 | bincode 디스크 덤프 | 서버 재시작 cold start 방지 | 2차 검증 |
| **[v3]** 정규화 | safe_normalize | 후보 1~2개 시 NaN/division-by-zero 방지 | 2차 검증 |
| **[v3]** Bandit 알고리즘 | Sliding Window UCB | 프로바이더 성능 변동에 빠른 적응 | 2차 검증 |
| **[v3]** PII 윈도우 | 동적 계산 (auto) | 가장 긴 PII 패턴 기준 + 마진 | 2차 검증 |
| **[v3]** Output Validation | 비율 기반 (0.3) | 짧은 시스템 프롬프트에서도 동작 | 2차 검증 |
| **[v3]** 스트리밍 버퍼 | 128K 토큰 상한 | 장문 응답 시 메모리 보호 | 2차 검증 |
| **[v3]** Health Check | deep/shallow 분리 | K8s liveness/readiness probe 대응 | 2차 검증 |
| **[v3]** CostForecaster | 신뢰구간 포함 | 예측 불확실성 투명 공개 | 2차 검증 |
| **[v3]** Store Migration | 번들 도구 제공 | SQLite→PG 전환 시 데이터 손실 방지 | 2차 검증 |
| **[v3]** CI/CD | GitHub Actions 5-stage | lint→unit→bench→e2e→docker 파이프라인 | 2차 검증 |
| *[v2]* 크레이트 수 | 3개 (Lean) | 소규모 팀 속도 확보 | 1차 검증 |
| *[v2]* 캐싱 전략 | 2단계 (Exact→Semantic) | 임베딩 10ms → <1ms 보존 | 1차 검증 |
| *[v2]* 비용 스코어링 | 정규화 + 컨텍스트 가중치 | 단위 불일치 해결 | 1차 검증 |
| *[v2]* PII 탐지 | 3단계 파이프라인 | Regex 오탐율 감소 | 1차 검증 |
| *[v2]* PII 방향 | 양방향 | 응답 PII 누출 방어 | 1차 검증 |
| *[v2]* 관측 저장소 | Tiered (SQLite 기본) | 초기 배포 부담 감소 | 1차 검증 |

---

## 부록: 용어 정리

- **Data Plane**: 실제 요청 처리 경로 (나노초 단위)
- **Control Plane**: 백그라운드 학습/갱신 경로
- **PipelineResult**: 비스트리밍/스트리밍 응답 구분 `[v2]`
- **StreamAssembler**: 스트리밍 응답 조립기 + 버퍼 상한 `[v3]`
- **TieredCache**: Exact(L1) + Semantic(L2) 2단계 캐시 `[v2]`
- **CacheWarmer**: 서버 재시작 시 캐시 사전 로딩 `[v3]`
- **CostAuctioneer**: 정규화 + 컨텍스트 가중치 경매 엔진 `[v2]`
- **safe_normalize**: 후보 1~2개 시 안전한 min-max 정규화 `[v3]`
- **SlidingWindowUCB**: 최근 N회만 사용하는 적응형 Bandit `[v3]`
- **ContextValidator**: PII 오탐 필터링 `[v2]`
- **SlidingBuffer**: 스트리밍 PII 탐지 윈도우 (동적 크기) `[v2]`+`[v3]`
- **PromptGuard**: Input Scan + Role Boundary + Output Validation(비율 기반) `[v2]`+`[v3]`
- **CostForecaster**: 월말 비용 예측 + 신뢰구간 `[v2]`+`[v3]`
- **StoreMigrator**: SQLite→PG 데이터 마이그레이션 도구 `[v3]`
- **HNSW**: 근사 최근접 이웃 탐색 알고리즘
- **GCRA**: 속도 제한 알고리즘
- **LFU**: 사용 빈도 최저 항목 축출 `[v2]`
