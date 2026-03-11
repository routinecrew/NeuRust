#!/bin/bash
# =============================================================
# NeuRust 에이전트 실행 스크립트
# =============================================================
# 사용법:
#   ./run-agents.sh a    → Agent A (core) 실행
#   ./run-agents.sh b    → Agent B (gateway) 실행
#   ./run-agents.sh c    → Agent C (cache & router) 실행
#   ./run-agents.sh d    → Agent D (security) 실행
#   ./run-agents.sh e    → Agent E (observability & store) 실행
#
# 실행 순서:
#   1단계: 터미널 1에서 → ./run-agents.sh a    (완료 대기)
#   2단계: 터미널 2에서 → ./run-agents.sh c
#          터미널 3에서 → ./run-agents.sh d
#          터미널 4에서 → ./run-agents.sh e
#   3단계: 터미널 5에서 → ./run-agents.sh b
# =============================================================

set -e
cd "$(dirname "$0")"

AGENT="$1"

CLAUDE_CMD="claude --dangerously-skip-permissions -p"

if [ -z "$AGENT" ]; then
  echo "사용법: ./run-agents.sh [a|b|c|d|e]"
  echo ""
  echo "  a  →  Agent A: neurust-core (Config, Pipeline, Provider, Auth)"
  echo "  b  →  Agent B: neurust-gateway (HTTP Server, OpenAI API, SSE)"
  echo "  c  →  Agent C: neurust-intel/cache+router (TieredCache, CostAuctioneer)"
  echo "  d  →  Agent D: neurust-intel/security (PII, PromptGuard)"
  echo "  e  →  Agent E: neurust-intel/observability+store (CostTracker, Store)"
  echo ""
  echo "권장 순서: a → (c, d, e 동시) → b"
  exit 1
fi

case "$AGENT" in
  a)
    echo "🚀 Agent A (neurust-core) 시작..."
    $CLAUDE_CMD "
당신은 Agent A입니다. NeuRust 프로젝트의 코어 엔진을 만듭니다.

먼저 다음 파일들을 읽으세요:
- skills/agent-a-core.md (당신의 스킬)
- contracts/shared_types.rs (공유 타입 계약)
- NeuRust_System_Design.md (시스템 설계서)
- CLAUDE.md (프로젝트 규칙)

crates/neurust-core/ 디렉토리에 소스코드를 만들어주세요.
Cargo.toml은 이미 존재합니다. 수정이 필요하면 수정하세요.

구현 순서:
1. contracts/shared_types.rs의 타입들을 neurust-core/src/contracts.rs로 복사하여 re-export
2. contracts/mock.rs를 neurust-core/src/mock.rs로 복사
3. Error 타입 정의 (error.rs)
4. Config 파싱 (config.rs) — serde_yaml로 NeuRustConfig 로딩 + 핫 리로드
5. Provider 시스템 (provider/) — Provider trait 구현체 (OpenAI, Anthropic, OpenAI-Compatible)
6. Pipeline (pipeline.rs) — PipelineLayer 미들웨어 체인
7. Router (router/) — LoadBalancer, Fallback, HealthMonitor
8. Auth (auth/) — API Key 검증 + Rate Limiting

각 단계마다 cargo test -p neurust-core가 통과하게 해주세요.
unwrap() 금지, println! 금지, tracing 사용.
"
    ;;

  b)
    echo "🚀 Agent B (neurust-gateway) 시작..."
    $CLAUDE_CMD "
당신은 Agent B입니다. NeuRust 프로젝트의 HTTP Gateway 서버를 만듭니다.

먼저 다음 파일들을 읽으세요:
- skills/agent-b-gateway.md (당신의 스킬)
- contracts/shared_types.rs (공유 타입 계약)
- contracts/mock.rs (Mock 구현)
- CLAUDE.md (프로젝트 규칙)

crates/neurust-gateway/ 디렉토리에서 작업하세요.
Cargo.toml은 이미 존재합니다. 수정이 필요하면 수정하세요.

neurust-core 의존성이 있지만, mock을 활용하여 독립 테스트도 가능합니다.
contracts/shared_types.rs의 타입과 contracts/mock.rs의 MockProvider를
크레이트 내부에 복사하여 사용하세요.

구현 순서:
1. 공유 타입/mock을 src/contracts.rs, src/mock.rs로 복사
2. axum 서버 구성 (server.rs) — Graceful Shutdown 포함
3. OpenAI 호환 라우트 (routes/inference.rs) — POST /v1/chat/completions
4. 모델 목록 (routes/models.rs) — GET /v1/models
5. 헬스체크 (routes/health.rs) — GET /health/live, /health/ready
6. 관리 API (routes/admin.rs) — GET/PATCH /admin/config, SSE 이벤트
7. 미들웨어 (middleware/) — Auth, RequestId, ErrorHandler
8. SSE 스트리밍 프록시 (sse.rs) — PipelineResult::Stream 변환
9. 단위 테스트

cargo test -p neurust-gateway가 통과해야 합니다.
unwrap() 금지, println! 금지, tracing 사용.
"
    ;;

  c)
    echo "🚀 Agent C (neurust-intel: cache & router) 시작..."
    $CLAUDE_CMD "
당신은 Agent C입니다. NeuRust 프로젝트의 지능형 캐싱과 비용 최적화 라우팅을 만듭니다.

먼저 다음 파일들을 읽으세요:
- skills/agent-c-cache.md (당신의 스킬)
- contracts/shared_types.rs (공유 타입 계약)
- contracts/mock.rs (Mock 구현)
- CLAUDE.md (프로젝트 규칙)

crates/neurust-intel/ 디렉토리에서 작업하세요.
src/cache/ 와 src/router/ 디렉토리가 당신의 영역입니다.
다른 디렉토리(security/, observability/, store/)는 Agent D, E의 영역이니 건드리지 마세요.

neurust-core 의존성이 있지만, mock을 활용하여 독립 테스트도 가능합니다.

구현 순서:
1. 공유 타입/mock을 src/contracts.rs, src/mock.rs로 복사 (이미 stub이 있으면 내용 채우기)
2. lib.rs에 cache, router 모듈 선언 추가
3. Exact Cache (cache/exact_cache.rs) — DashMap + XxHash64 + [v3] 턴 수 포함 해시
4. Cache Key (cache/cache_key.rs) — 멀티턴 캐시 키 생성
5. Cache Eviction (cache/eviction.rs) — LFU 축출
6. Cache Warming (cache/warming.rs) — [v3] bincode 디스크 덤프/로드
7. TieredCache (cache/mod.rs) — PipelineLayer 구현
8. CostAuctioneer (router/cost_auctioneer.rs) — [v3] safe_normalize
9. BanditRouter (router/bandit_router.rs) — [v3] Sliding Window UCB
10. ComplexityGate (router/complexity_gate.rs) — 요청 복잡도 분류
11. Price DB (router/price_db.rs) — 모델 가격 로딩
12. 단위 테스트

cargo test -p neurust-intel가 통과해야 합니다.
unwrap() 금지, println! 금지, tracing 사용.
"
    ;;

  d)
    echo "🚀 Agent D (neurust-intel: security) 시작..."
    $CLAUDE_CMD "
당신은 Agent D입니다. NeuRust 프로젝트의 보안 레이어를 만듭니다.

먼저 다음 파일들을 읽으세요:
- skills/agent-d-security.md (당신의 스킬)
- contracts/shared_types.rs (공유 타입 계약)
- contracts/mock.rs (Mock 구현)
- CLAUDE.md (프로젝트 규칙)

crates/neurust-intel/ 디렉토리에서 작업하세요.
src/security/ 디렉토리가 당신의 영역입니다.
다른 디렉토리(cache/, router/, observability/, store/)는 Agent C, E의 영역이니 건드리지 마세요.

구현 순서:
1. lib.rs에 security 모듈 선언 추가 (조건부: #[cfg(feature = \"security\")])
2. PiiDetector (security/pii_detector.rs) — Regex + Aho-Corasick 3단계 탐지
3. PiiMasker (security/pii_masker.rs) — 양방향 마스킹 + [v3] 동적 윈도우
4. ContextValidator (security/context_validator.rs) — 오탐 필터링
5. PromptGuard (security/prompt_guard.rs) — 3중 방어 + [v3] 비율 기반 Output Validation
6. AuditLogger (security/audit_logger.rs) — 감사 로그
7. PipelineLayer 구현 (security/mod.rs) — PiiMaskerLayer + PromptGuardLayer
8. 단위 테스트

cargo test -p neurust-intel --features security가 통과해야 합니다.
unwrap() 금지, println! 금지, tracing 사용.
"
    ;;

  e)
    echo "🚀 Agent E (neurust-intel: observability & store) 시작..."
    $CLAUDE_CMD "
당신은 Agent E입니다. NeuRust 프로젝트의 관측성과 데이터 저장소를 만듭니다.

먼저 다음 파일들을 읽으세요:
- skills/agent-e-observability.md (당신의 스킬)
- contracts/shared_types.rs (공유 타입 계약)
- contracts/mock.rs (Mock 구현)
- CLAUDE.md (프로젝트 규칙)

crates/neurust-intel/ 디렉토리에서 작업하세요.
src/observability/ 와 src/store/ 디렉토리가 당신의 영역입니다.
다른 디렉토리(cache/, router/, security/)는 Agent C, D의 영역이니 건드리지 마세요.

구현 순서:
1. lib.rs에 observability, store 모듈 선언 추가
2. CostTracker (observability/cost_tracker.rs) — 요청별 비용 계산
3. CostForecaster (observability/cost_forecaster.rs) — [v3] 신뢰구간 포함 월말 예측
4. BudgetManager (observability/budget_manager.rs) — 예산 관리 + 알림
5. MetricsCollector (observability/metrics.rs) — Prometheus 형식 메트릭
6. SQLite Store (store/sqlite_store.rs) — EventStore trait 구현
7. PostgreSQL Store (store/postgres_store.rs) — EventStore trait 구현 (stub)
8. Store Migrator (store/migrator.rs) — [v3] SQLite → PG 마이그레이션
9. Store 모듈 (store/mod.rs) — 자동 스토어 선택
10. PipelineLayer 구현 (observability/mod.rs) — ObservabilityLayer
11. 단위 테스트

cargo test -p neurust-intel --features 'observability,store-sqlite'가 통과해야 합니다.
unwrap() 금지, println! 금지, tracing 사용.
"
    ;;

  *)
    echo "❌ 알 수 없는 에이전트: $AGENT"
    echo "사용법: ./run-agents.sh [a|b|c|d|e]"
    exit 1
    ;;
esac
