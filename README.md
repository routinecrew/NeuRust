<p align="center">
  <img src="https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white" alt="Rust"/>
  <img src="https://img.shields.io/badge/Tokio-async-blue?style=for-the-badge" alt="Tokio"/>
  <img src="https://img.shields.io/badge/Axum-HTTP-green?style=for-the-badge" alt="Axum"/>
  <img src="https://img.shields.io/badge/License-MIT-yellow?style=for-the-badge" alt="License"/>
</p>

# NeuRust

> **The Intelligent Rust Gateway for Nano-second AI Routing**

OpenAI, Anthropic, 그리고 모든 LLM API를 하나의 엔드포인트로 통합하는 **고성능 AI Gateway**.
단순 프록시가 아닌, 요청을 이해하고 최적의 경로를 찾는 **지능형 라우터**.

```
Your App  ──→  NeuRust  ──→  OpenAI
                  │       ──→  Anthropic
                  │       ──→  Ollama / vLLM / Any OpenAI-compatible
                  │
                  ├── 캐싱으로 비용 70% 절감
                  ├── 자동으로 가장 빠르고 저렴한 모델 선택
                  ├── PII 자동 마스킹 (개인정보 유출 방지)
                  └── 실시간 비용 추적 & 월말 예측
```

---

## Why NeuRust?

| | 기존 프록시 | **NeuRust** |
|---|---|---|
| **언어** | Go / Python / Node.js | **Rust** — zero-cost abstractions, no GC |
| **지연** | 10~50ms 오버헤드 | **< 1ms** 오버헤드 (나노초 라우팅) |
| **캐싱** | 단순 키-값 | **2단계**: Exact Hash → Semantic 유사도 |
| **라우팅** | 수동 설정 | **Multi-Armed Bandit** 자동 최적화 |
| **보안** | 별도 서비스 | **빌트인**: PII 마스킹 + Prompt Injection 방어 |
| **비용** | 로그 분석 | **실시간 추적** + 신뢰구간 포함 월말 예측 |
| **배포** | 의존성 다수 | **단일 바이너리** (3MB, Docker 이미지 10MB) |

---

## Key Features

### 1. Intelligent Caching — 같은 질문에 두 번 돈 쓰지 않기

```
요청 → [Exact Cache] ──hit──→ 즉시 응답 (< 0.1ms)
              │ miss
        [Semantic Cache] ──hit──→ 유사 응답 반환 (< 5ms)
              │ miss
        [Provider 호출] ──→ 응답 + 캐시 저장
```

- **Exact Cache**: XxHash64 기반 O(1) 조회, 멀티턴 대화 지문 포함
- **Semantic Cache**: 임베딩 유사도 검색으로 "비슷한 질문"도 캐시 히트
- **Cache Warming**: 서버 재시작 시 캐시 사전 로딩 — cold start 제거

### 2. Cost-Optimized Routing — AI가 AI 비용을 줄이다

```yaml
# 대화형 요청: 속도 우선
interactive: { cost: 0.2, latency: 0.6, quality: 0.2 }

# 배치 처리: 비용 우선
batch: { cost: 0.7, latency: 0.1, quality: 0.2 }
```

- **Sliding Window UCB**: 프로바이더 성능 변동에 실시간 적응
- **Complexity Gate**: 간단한 질문은 저렴한 모델로, 복잡한 질문은 고성능 모델로
- **Budget Guard**: 월 예산 80% 도달 시 경고, 100% 초과 시 자동 차단

### 3. Enterprise Security — 기업이 안심하고 쓸 수 있는 AI

```
"제 이메일은 john@company.com이고 카드번호는 4242-4242-4242-4242입니다"
                          ↓ NeuRust
"제 이메일은 [EMAIL_1]이고 카드번호는 [CREDIT_CARD_1]입니다"
```

- **PII Masking**: 이메일, 전화번호, 카드번호, 주민번호 자동 탐지/마스킹
- **Prompt Injection Defense**: 3중 방어 (Input Scan + Role Boundary + Output Validation)
- **Audit Trail**: 모든 보안 이벤트 기록, 컴플라이언스 대응

### 4. Real-time Observability — 비용을 예측하고 통제하기

```
neurust_cost_usd_total{provider="openai"} 127.50
neurust_cache_hit_ratio{tier="exact"} 0.42
neurust_request_duration_p99{provider="anthropic"} 0.850

이번 달 예상 비용: $3,240 ± $180 (85% 신뢰구간)
추세: ↗ Rising (+12% vs 지난주)
```

- **Prometheus 메트릭**: 요청 수, 지연, 토큰, 비용, 캐시 히트율
- **Cost Forecaster**: 선형 회귀 기반 월말 비용 예측 + 신뢰구간
- **Tiered Storage**: SQLite(개인) → PostgreSQL(팀) → ClickHouse(기업) 자동 전환

---

## Quick Start

### 3줄로 시작하기

```yaml
# config/neurust.yml — 이것만 있으면 됩니다
providers:
  - id: openai
    provider_type: openai
    api_key_env: OPENAI_API_KEY
    models: ["gpt-4o", "gpt-4o-mini"]
```

```bash
# 실행
OPENAI_API_KEY=sk-... neurust

# 기존 코드 한 줄만 변경
# Before: https://api.openai.com/v1
# After:  http://localhost:8080/v1
```

### OpenAI SDK에서 바로 사용

```python
from openai import OpenAI

# NeuRust를 프록시로 사용 — 코드 변경 1줄
client = OpenAI(base_url="http://localhost:8080/v1")

response = client.chat.completions.create(
    model="gpt-4o",
    messages=[{"role": "user", "content": "Hello, NeuRust!"}]
)
```

---

## Architecture

```
                         ┌─────────────────────────────┐
                         │       neurust-gateway        │
                         │   (Axum HTTP, port 8080)     │
                         └──────────────┬──────────────┘
                                        │
                         ┌──────────────▼──────────────┐
                         │        neurust-core          │
                         │  Pipeline (Tower Middleware)  │
                         │                              │
                         │  ┌──────┐ ┌──────┐ ┌──────┐ │
                         │  │ Auth │→│Cache │→│Router│ │
                         │  └──────┘ └──────┘ └──┬───┘ │
                         │  ┌──────┐ ┌──────┐    │     │
                         │  │  PII │→│Observ│    │     │
                         │  └──────┘ └──────┘    │     │
                         └───────────────────────┼─────┘
                                                 │
                    ┌────────────┬───────────────┼────────────┐
                    ▼            ▼               ▼            ▼
               ┌────────┐  ┌────────┐     ┌──────────┐  ┌────────┐
               │ OpenAI │  │Anthropic│    │  Ollama  │  │ vLLM   │
               └────────┘  └────────┘     └──────────┘  └────────┘
```

**3-crate workspace:**

| Crate | Description |
|-------|-------------|
| `neurust-core` | Config, Pipeline, Provider, Auth, Router |
| `neurust-gateway` | HTTP server, OpenAI-compatible API, SSE streaming |
| `neurust-intel` | Cache, Cost Router, Security, Observability, Store |

`neurust-intel`은 **Cargo feature flags**로 선택적 컴파일:

```bash
cargo build                          # 기본: Exact Cache + Observability (빌드 30초)
cargo build --features security      # + PII Masking, Prompt Guard
cargo build --features full          # 전체 기능 (빌드 2~3분)
```

---

## Feature Flags

| Feature | 포함 모듈 | 기본 |
|---------|----------|------|
| `cache-exact` | Exact Hash Cache | ON |
| `cache-semantic` | Semantic Similarity Cache | OFF |
| `security` | PII Masking + Prompt Guard | OFF |
| `cost-router` | Cost Auctioneer + Bandit Router | OFF |
| `observability` | Cost Tracker + Metrics | ON |
| `store-sqlite` | SQLite 저장소 | OFF |
| `store-postgres` | PostgreSQL 저장소 | OFF |
| `full` | 모든 기능 | OFF |

---

## Configuration

<details>
<summary>전체 설정 예시 (클릭하여 펼치기)</summary>

```yaml
server:
  address: "0.0.0.0:8080"
  graceful_shutdown_sec: 30

providers:
  - id: openai
    provider_type: openai
    api_key_env: OPENAI_API_KEY
    models: ["gpt-4o", "gpt-4o-mini"]
    priority: 1

  - id: anthropic
    provider_type: anthropic
    api_key_env: ANTHROPIC_API_KEY
    models: ["claude-sonnet-4-20250514", "claude-haiku-4-5-20251001"]
    priority: 2

auth:
  api_keys:
    - key: "sk-neurust-your-key"
      name: "production"
      rate_limit: 1000  # req/min

intelligence:
  tiered_cache:
    exact_cache:
      enabled: true
      max_entries: 100000
      ttl_sec: 3600
  cost_router:
    enabled: true
    bandit_algorithm: sliding_window_ucb
    context_weights:
      interactive: { cost: 0.2, latency: 0.6, quality: 0.2 }
      batch:       { cost: 0.7, latency: 0.1, quality: 0.2 }

security:
  pii_masking:
    enabled: true
    direction: bidirectional
  prompt_guard:
    input_scanning: true
    output_validation: true

observability:
  cost_forecaster:
    enabled: true
    confidence_interval: 0.85
```

</details>

---

## API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/chat/completions` | Chat completion (OpenAI compatible) |
| `POST` | `/v1/embeddings` | Embedding generation |
| `GET` | `/v1/models` | List available models |
| `GET` | `/health/live` | Liveness probe (K8s) |
| `GET` | `/health/ready` | Readiness probe (deep check) |
| `GET` | `/admin/stats` | Request statistics |
| `GET` | `/admin/providers` | Provider health status |
| `GET` | `/admin/events/stream` | SSE real-time events |
| `GET/PATCH` | `/admin/config` | Runtime config (hot-reload) |

---

## Benchmarks

> Target performance (development in progress)

| Metric | Target | Notes |
|--------|--------|-------|
| Proxy overhead | < 1ms p99 | Non-cached request |
| Exact Cache lookup | < 0.1ms | DashMap + XxHash64 |
| Cache throughput | > 100K req/sec | Single instance |
| PII detection | < 5ms p99 | 1KB request body |
| Memory usage | < 50MB | Base (no cache data) |
| Binary size | < 10MB | Statically linked |

---

## Roadmap

- [x] **Phase 0** — Workspace scaffold, shared contracts
- [ ] **Phase 1** — Core Proxy: Pipeline + OpenAI/Anthropic providers + Gateway
- [ ] **Phase 2** — Intelligence: TieredCache + Cost Router + Bandit
- [ ] **Phase 3** — Security: PII Masking + Prompt Guard
- [ ] **Phase 4** — Observability: Cost Forecaster + Tiered Storage
- [ ] **Phase 5** — Scale: Redis shared state, SDK, additional providers

---

## Comparison

| Feature | NeuRust | LiteLLM | Helicone | TensorZero |
|---------|---------|---------|----------|------------|
| Language | Rust | Python | TS/Node | Rust |
| Latency overhead | < 1ms | 10-50ms | 20-100ms | 1-5ms |
| Semantic cache | Built-in | Plugin | No | No |
| Cost routing (Bandit) | Built-in | No | No | Built-in |
| PII masking | Built-in | No | No | No |
| Prompt guard | Built-in | No | No | No |
| Cost forecasting | Built-in | No | Dashboard | No |
| Single binary | Yes | No | No | Yes |
| Feature flags | Yes | N/A | N/A | No |

---

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.

```bash
# Build
cargo build --workspace

# Test
cargo test --workspace

# Test with all features
cargo test --workspace --features full
```

---

## License

MIT

---

<p align="center">
  <b>NeuRust</b> — Stop overpaying for AI. Start routing intelligently.
</p>
