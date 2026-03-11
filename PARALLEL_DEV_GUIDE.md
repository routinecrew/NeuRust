# NeuRust — 병렬 개발 가이드

> 5개 에이전트가 동시에 개발하기 위한 운영 매뉴얼

---

## 1. 전체 구조 요약

```
                    ┌──────────────────────────────────────┐
                    │       contracts/shared_types.rs       │
                    │  (모든 에이전트가 공유하는 타입/trait)   │
                    └──────────────────┬───────────────────┘
                                       │
           ┌───────────────────────────┼────────────────────────────┐
           │                           │                            │
     ┌─────▼──────┐            ┌──────▼────────┐           ┌──────▼──────────┐
     │   Agent A   │            │   Agent B     │           │   Agent C/D/E   │
     │neurust-core │            │neurust-gateway│           │  neurust-intel  │
     │(기반 계층)   │            │(HTTP 진입점)  │           │(지능/보안/관측)  │
     └─────┬──────┘            └───────────────┘           └─────────────────┘
           │                                                       │
           │          Pipeline + Provider                          │
           ├──────────────────────────────────────────────────────┤
           │                                                       │
     ┌─────▼──────┐  ┌─────▼──────┐  ┌──────▼──────┐  ┌─────▼─────┐
     │  Cache &    │  │  Security  │  │Observability│  │   Store   │
     │Cost Router  │  │  PII/Guard │  │  Cost/Metric│  │ SQLite/PG │
     │  (Agent C)  │  │ (Agent D)  │  │  (Agent E)  │  │ (Agent E) │
     └─────────────┘  └────────────┘  └─────────────┘  └───────────┘
```

---

## 2. 에이전트 역할 배정

| 에이전트 | 크레이트/모듈 | 핵심 역할 | 스킬 파일 |
|----------|-------------|----------|-----------|
| **Agent A** | `neurust-core` | Config, Pipeline, Provider, Auth, Router | `skills/agent-a-core.md` |
| **Agent B** | `neurust-gateway` | HTTP Server, OpenAI 호환 API, SSE | `skills/agent-b-gateway.md` |
| **Agent C** | `neurust-intel` cache/ + router/ | TieredCache, CostAuctioneer, Bandit | `skills/agent-c-cache.md` |
| **Agent D** | `neurust-intel` security/ | PII, PromptGuard, AuditLogger | `skills/agent-d-security.md` |
| **Agent E** | `neurust-intel` observability/ + store/ | CostTracker, Forecaster, Store | `skills/agent-e-observability.md` |

---

## 3. 의존성 그래프와 병렬화 전략

```
Week 1-2:  [A: core 기본]  [C: cache 독립]  [D: security 독립]  [E: store 독립]
               │
Week 3-4:  [A: core 완성] ──▶ [B: gateway 시작]
               │                    │
Week 5-6:  [B: gateway 완성]  [C: 통합]  [D: 통합]  [E: 통합]
               │                    │         │          │
Week 7-8:  ◀──────── 전체 통합 테스트 + 벤치마크 ────────────▶
```

### 핵심 원칙: Mock으로 독립 개발

Agent A(core)가 완성되기 전에도 B, C, D, E는 **mock**을 써서 동시 개발한다.

```rust
// 모든 에이전트가 사용하는 공통 mock (contracts/mock.rs)
let provider = MockProvider::openai();
let response = provider.complete(&request).await?;

let store = MockEventStore::new();
store.record_cost(&event).await?;
```

**이것이 `Provider` trait과 `EventStore` trait으로 인터페이스를 분리한 이유다.**
mock만 교체하면 실제 core 없이 각 모듈을 독립 실행할 수 있다.

---

## 4. Claude Code 에이전트 실행 방법

### 4.1 사전 준비

```bash
cd neurust
cargo build --workspace  # 최초 빌드 확인
```

### 4.2 에이전트 실행

```bash
# 터미널 1 — Agent A: Core
./run-agents.sh a

# 터미널 2 — Agent B: Gateway
./run-agents.sh b

# 터미널 3 — Agent C: Cache & Router
./run-agents.sh c

# 터미널 4 — Agent D: Security
./run-agents.sh d

# 터미널 5 — Agent E: Observability & Store
./run-agents.sh e
```

---

## 5. 통합 순서

### Phase 1: Core + Gateway (가장 먼저 통합)
```bash
# Agent A의 neurust-core가 준비되면
# Agent B가 MockProvider/Pipeline → 실제 Pipeline으로 교체

# 검증:
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer test-key" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-4o","messages":[{"role":"user","content":"Hello"}]}'
```

### Phase 2: Core + Intel Cache (캐시 연결)
```bash
# Agent C가 TieredCacheLayer를 Pipeline에 삽입
# 동일 요청 2회 → 2번째는 캐시 히트

# 검증:
# 같은 요청 두 번 → 두 번째 응답 지연 < 1ms
```

### Phase 3: Core + Intel Security (보안 연결)
```bash
# Agent D가 PiiMaskerLayer, PromptGuardLayer를 Pipeline에 삽입

# 검증:
# PII 포함 요청 → 마스킹 확인
# Injection 시도 → 차단 확인
```

### Phase 4: Core + Intel Observability (관측 연결)
```bash
# Agent E가 ObservabilityLayer를 Pipeline에 삽입

# 검증:
curl http://localhost:8080/admin/stats
curl http://localhost:8080/admin/events/stream  # SSE
```

### Phase 5: 전체 통합
```
Client → Gateway → Auth → PII Masker → Cache Check
                                          │ miss
                                    Cost Router → Provider
                                          │
                              PromptGuard ← Response
                                          │
                              Cache Store → Observability → Client
```

---

## 6. 충돌 방지 규칙

### 6.1 파일 소유권

| 디렉토리/파일 | 소유 에이전트 | 다른 에이전트 접근 |
|--------------|-------------|-----------------|
| `crates/neurust-core/` | Agent A | 읽기만 |
| `crates/neurust-gateway/` | Agent B | 읽기만 |
| `crates/neurust-intel/src/cache/` | Agent C | 읽기만 |
| `crates/neurust-intel/src/router/` | Agent C | 읽기만 |
| `crates/neurust-intel/src/security/` | Agent D | 읽기만 |
| `crates/neurust-intel/src/observability/` | Agent E | 읽기만 |
| `crates/neurust-intel/src/store/` | Agent E | 읽기만 |
| `contracts/` | **공동 소유** | 변경 시 PR 필수 |
| `NeuRust_System_Design.md` | **공동 소유** | 변경 시 전체 공유 |

### 6.2 neurust-intel 공유 규칙
Agent C, D, E가 동일 크레이트(neurust-intel)에서 작업하므로:
- `lib.rs`는 모듈 선언만 추가 (충돌 최소화)
- 각자 담당 디렉토리 내에서만 파일 생성/수정
- `Cargo.toml`의 features/dependencies 변경 시 다른 에이전트에게 알림
- `contracts.rs`, `mock.rs`는 공통 파일이므로 Agent C가 초기 설정, 이후 변경 시 협의

### 6.3 contracts 변경 프로토콜
1. 변경이 필요한 에이전트가 `contracts/shared_types.rs` 수정 PR 생성
2. PR 설명에 "영향받는 에이전트: B, D" 등 명시
3. 다른 에이전트가 확인 후 자기 크레이트 업데이트
4. 모든 크레이트 `cargo test` 통과 확인 후 merge

### 6.4 Git 브랜치 전략

```
main ─────────────────────────────────────────▶
  │
  ├── agent-a/core ────── Agent A 작업 ──────── PR → main
  ├── agent-b/gateway ─── Agent B 작업 ──────── PR → main
  ├── agent-c/intel ───── Agent C 작업 ──────── PR → main
  ├── agent-d/security ── Agent D 작업 ──────── PR → main
  └── agent-e/observe ─── Agent E 작업 ──────── PR → main
```

---

## 7. 체크리스트

### Week 1-2 체크리스트
- [ ] contracts/shared_types.rs 확정
- [ ] Cargo workspace 빌드 성공
- [ ] Agent A: Config 파싱 + Provider trait 구현 + Pipeline 동작
- [ ] Agent C: ExactCache 독립 구현 + 테스트
- [ ] Agent D: PiiDetector Regex 탐지 + 테스트
- [ ] Agent E: CostTracker + MockEventStore 테스트

### Week 3-4 체크리스트
- [ ] Agent A: OpenAI/Anthropic Provider + Fallback + Auth
- [ ] Agent B: axum 서버 + OpenAI 호환 API + SSE 스트리밍
- [ ] Agent C: CostAuctioneer + BanditRouter + safe_normalize
- [ ] Agent D: PiiMasker 양방향 + PromptGuard 3중 방어
- [ ] Agent E: CostForecaster 신뢰구간 + SQLite Store

### Week 5-6 체크리스트
- [ ] Core + Gateway 통합 성공
- [ ] Core + Cache 통합 (캐시 히트 확인)
- [ ] Core + Security 통합 (PII 마스킹 확인)
- [ ] Core + Observability 통합 (비용 추적 확인)

### Week 7-8 체크리스트
- [ ] 전체 통합 테스트 통과
- [ ] 성능 벤치마크 (p99 지연, 캐시 처리량)
- [ ] OpenAI Python SDK로 End-to-End 테스트
- [ ] Docker 이미지 빌드
- [ ] CI/CD 파이프라인 동작

---

## 8. 트러블슈팅

### "다른 에이전트의 모듈이 컴파일 안 돼요"
→ 자기 모듈만 테스트: `cargo test -p neurust-intel --features security`
→ 다른 모듈 의존은 mock으로 대체

### "contracts 타입이 부족해요"
→ contracts 변경 PR 생성 → 다른 에이전트에게 리뷰 요청
→ 임시로 자기 모듈 내부에 확장 타입 정의 (나중에 contracts로 이동)

### "neurust-intel에서 Agent C, D, E가 충돌해요"
→ 각자 담당 디렉토리에서만 작업
→ lib.rs는 모듈 선언만 추가 (한 줄씩이므로 충돌 가능성 낮음)
→ Cargo.toml 변경은 PR로 조율

### "feature flag 조합에서 컴파일 에러"
→ `cargo test --features full`로 전체 조합 테스트
→ `#[cfg(feature = "...")]` 경계 확인
