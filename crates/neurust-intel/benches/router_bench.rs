//! Benchmarks for CostAuctioneer scoring and SlidingWindowUCB bandit operations.

use std::collections::HashMap;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use tokio::runtime::Runtime;

use neurust_intel::contracts::{
    CostRouterConfig, Message, ModelSpec, RequestContext, Role, UnifiedRequest,
};
use neurust_intel::router::bandit::SlidingWindowUCB;
use neurust_intel::router::cost_auctioneer::{safe_normalize, CostAuctioneer, RawScore};

/// Create a test request for benchmarking.
fn make_request() -> UnifiedRequest {
    UnifiedRequest {
        model: ModelSpec {
            model_name: "gpt-4o".to_string(),
            provider_id: None,
        },
        messages: vec![Message {
            role: Role::User,
            content: "Benchmark test message for cost routing evaluation".to_string(),
        }],
        temperature: Some(0.7),
        max_tokens: Some(100),
        stream: false,
        context: RequestContext {
            request_type: Some("interactive".to_string()),
            budget_remaining_ratio: 0.8,
            client_id: Some("bench-client".to_string()),
            api_key: None,
        },
        extra_params: HashMap::new(),
    }
}

/// Create N candidate RawScores with varied cost/latency/quality.
fn make_candidates(n: usize) -> Vec<RawScore> {
    (0..n)
        .map(|i| RawScore {
            model_id: format!("model-{}", i),
            cost: 0.01 + (i as f64 * 0.005),
            latency_ms: 50.0 + (i as f64 * 30.0),
            quality: 0.95 - (i as f64 * 0.03),
        })
        .collect()
}

/// Create auctioneer in warmed-up state (past warmup threshold).
fn make_warmed_auctioneer(candidates: Vec<RawScore>) -> CostAuctioneer {
    let config = CostRouterConfig {
        enabled: true,
        warmup_requests: Some(0), // Skip warmup for scoring benchmarks
        epsilon: Some(0.0),       // Disable random exploration for determinism
        normalize_mode: None,
        bandit_algorithm: None,
        sliding_window_size: Some(500),
        context_weights: None,
    };
    CostAuctioneer::new(config, candidates)
}

fn bench_auctioneer_select_best(c: &mut Criterion) {
    let rt = Runtime::new().expect("failed to create runtime");
    let mut group = c.benchmark_group("auctioneer_select_best");

    for n_candidates in [3, 5, 10, 20] {
        let candidates = make_candidates(n_candidates);
        let auctioneer = make_warmed_auctioneer(candidates);
        let request = make_request();

        group.bench_with_input(
            BenchmarkId::new("candidates", n_candidates),
            &n_candidates,
            |b, _| {
                b.iter(|| {
                    rt.block_on(async {
                        black_box(auctioneer.select_best(black_box(&request)).await)
                    })
                });
            },
        );
    }

    group.finish();
}

fn bench_auctioneer_cold_start(c: &mut Criterion) {
    let rt = Runtime::new().expect("failed to create runtime");
    let candidates = make_candidates(5);

    let config = CostRouterConfig {
        enabled: true,
        warmup_requests: Some(1000), // High warmup -- always cold start path
        epsilon: Some(0.0),
        normalize_mode: None,
        bandit_algorithm: None,
        sliding_window_size: Some(500),
        context_weights: None,
    };
    let auctioneer = CostAuctioneer::new(config, candidates);
    let request = make_request();

    c.bench_function("auctioneer_cold_start_select", |b| {
        b.iter(|| {
            rt.block_on(async { black_box(auctioneer.select_best(black_box(&request)).await) })
        });
    });
}

fn bench_safe_normalize(c: &mut Criterion) {
    let mut group = c.benchmark_group("safe_normalize");

    for n in [3, 10, 50, 100] {
        let values: Vec<f64> = (0..n).map(|i| i as f64 * 1.5 + 0.1).collect();
        let mid_value = values[n / 2];

        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| black_box(safe_normalize(black_box(mid_value), black_box(&values))));
        });
    }

    // Edge case: all identical values
    let identical = vec![42.0; 100];
    group.bench_function("identical_100", |b| {
        b.iter(|| black_box(safe_normalize(black_box(42.0), black_box(&identical))));
    });

    group.finish();
}

fn bench_ucb_update(c: &mut Criterion) {
    let mut group = c.benchmark_group("ucb_update");

    for window_size in [100, 500, 1000] {
        group.bench_with_input(
            BenchmarkId::new("window", window_size),
            &window_size,
            |b, &ws| {
                b.iter_batched(
                    || SlidingWindowUCB::new(ws),
                    |mut bandit| {
                        for i in 0..100u64 {
                            let model = format!("model-{}", i % 5);
                            bandit.update(black_box(&model), black_box(0.7));
                        }
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn bench_ucb_get_score(c: &mut Criterion) {
    let mut group = c.benchmark_group("ucb_get_score");

    for n_arms in [3, 10, 50] {
        // Pre-populate bandit
        let mut bandit = SlidingWindowUCB::new(500);
        for arm in 0..n_arms {
            let model = format!("model-{}", arm);
            for _ in 0..100 {
                bandit.update(&model, 0.5 + (arm as f64 * 0.05));
            }
        }

        group.bench_with_input(
            BenchmarkId::new("arms", n_arms),
            &n_arms,
            |b, &n_arms| {
                let mut idx = 0;
                b.iter(|| {
                    let model = format!("model-{}", idx % n_arms);
                    idx += 1;
                    black_box(bandit.get_score(black_box(&model)))
                });
            },
        );
    }

    group.finish();
}

fn bench_ucb_select_record_cycle(c: &mut Criterion) {
    c.bench_function("ucb_select_record_cycle", |b| {
        b.iter_batched(
            || {
                let mut bandit = SlidingWindowUCB::new(500);
                // Warm up with initial data
                for arm in 0..5 {
                    for _ in 0..20 {
                        bandit.update(&format!("model-{}", arm), 0.5);
                    }
                }
                bandit
            },
            |mut bandit| {
                // Simulate realistic cycle: score all arms, pick best, record reward
                for _ in 0..50 {
                    let mut best_model = String::new();
                    let mut best_score = f64::NEG_INFINITY;
                    for arm in 0..5 {
                        let model = format!("model-{}", arm);
                        let score = bandit.get_score(&model);
                        if score > best_score {
                            best_score = score;
                            best_model = model;
                        }
                    }
                    bandit.update(black_box(&best_model), black_box(0.8));
                }
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

criterion_group!(
    benches,
    bench_auctioneer_select_best,
    bench_auctioneer_cold_start,
    bench_safe_normalize,
    bench_ucb_update,
    bench_ucb_get_score,
    bench_ucb_select_record_cycle,
);
criterion_main!(benches);
