# NeuRust — Claude Code 지침

## 프로젝트 개요
AI Gateway를 Rust로 구축하는 프로젝트.
OpenAI/Anthropic 등 LLM 프로바이더에 대한 지능형 프록시로,
캐싱, 비용 최적화 라우팅, PII 마스킹, 관측성을 제공한다.

## 반드시 읽어야 할 파일 (우선순위 순)
1. `contracts/shared_types.rs` — 공유 타입/trait. **절대 임의 변경 금지.**
2. `contracts/mock.rs` — 독립 개발용 mock 구현
3. `NeuRust_System_Design.md` — 전체 아키텍처 설계서
4. `PARALLEL_DEV_GUIDE.md` — 병렬 개발 운영 가이드
5. `skills/agent-*.md` — 본인 담당 에이전트의 상세 스킬

## 에이전트 배정
| 에이전트 | 크레이트/모듈 | 역할 |
|----------|-------------|------|
| Agent A | `crates/neurust-core/` | Config, Pipeline, Provider, Auth, Router |
| Agent B | `crates/neurust-gateway/` | HTTP Gateway, Routes, Middleware, SSE |
| Agent C | `crates/neurust-intel/src/cache/`, `src/router/` | TieredCache, CostAuctioneer, Bandit |
| Agent D | `crates/neurust-intel/src/security/` | PII Masking, PromptGuard, AuditLog |
| Agent E | `crates/neurust-intel/src/observability/`, `src/store/` | CostTracker, Forecaster, Store |

## 코딩 규칙
- `unwrap()` 금지. 모든 에러는 `anyhow::Result`로 전파
- `println!` 금지. 로그는 `tracing` 크레이트 사용
- `unsafe` 사용 시 반드시 주석으로 안전성 근거 명시
- public API에는 doc comment 필수
- 테스트: 각 public 함수에 최소 1개 단위 테스트

## 빌드/테스트
```bash
# 전체 빌드
cargo build --workspace

# 전체 테스트
cargo test --workspace

# 특정 크레이트 테스트
cargo test -p neurust-core
cargo test -p neurust-gateway
cargo test -p neurust-intel
cargo test -p neurust-intel --features security
cargo test -p neurust-intel --features "observability,store-sqlite"
cargo test -p neurust-intel --features full

# 벤치마크
cargo bench -p neurust-intel
```

## contracts 변경 절차
1. 변경 필요성 설명과 함께 PR 생성
2. 영향받는 에이전트 목록 명시
3. 모든 크레이트 `cargo test --workspace` 통과 확인 후 merge

## 독립 개발 방법
core가 아직 없어도 `contracts/mock.rs`의 MockProvider, MockEventStore를 사용하면
각 크레이트/모듈을 독립적으로 빌드하고 테스트할 수 있다.
통합 시에만 mock → 실제 구현으로 교체.

## 토큰 최적화
- **서브에이전트(Agent tool) 사용 금지** — 직접 Glob, Grep, Read 등 기본 도구로 해결할 것
- **응답은 최소한으로** — 코드 변경 시 변경 사항만 간결히 설명
- **파일은 필요한 부분만 읽기** — `offset`/`limit` 활용
- **병렬 도구 호출 활용** — 독립적인 도구 호출은 한 번에 병렬 실행
- **탐색 전 추론 우선** — 파일 경로가 예측 가능하면 탐색 없이 바로 접근
