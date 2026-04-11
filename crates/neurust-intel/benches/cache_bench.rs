//! Benchmarks for ExactCache hot paths: insert, lookup (hit/miss), TTL expiry, LFU eviction.

use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};

use neurust_intel::cache::exact_cache::ExactCache;
use neurust_intel::contracts::{TokenUsage, UnifiedResponse};

/// Create a minimal `UnifiedResponse` for benchmarking.
fn make_response(content: &str) -> UnifiedResponse {
    UnifiedResponse {
        content: content.to_string(),
        model: "gpt-4o".to_string(),
        usage: TokenUsage::default(),
        provider_id: "openai".to_string(),
        latency_ms: 100,
        upstream_id: None,
    }
}

fn bench_cache_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_insert");

    for count in [100, 1_000, 10_000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &count,
            |b, &count| {
                b.iter_batched(
                    || ExactCache::new(count + 1000, Duration::from_secs(300)),
                    |cache| {
                        for i in 0..count as u64 {
                            cache.insert(black_box(i), make_response("cached response"));
                        }
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn bench_cache_lookup_hit(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_lookup_hit");

    for size in [100, 1_000, 10_000] {
        // Pre-populate cache
        let cache = ExactCache::new(size + 1000, Duration::from_secs(300));
        for i in 0..size as u64 {
            cache.insert(i, make_response("cached response"));
        }

        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            let mut idx = 0u64;
            b.iter(|| {
                let key = idx % size as u64;
                idx += 1;
                black_box(cache.get(black_box(key)))
            });
        });
    }

    group.finish();
}

fn bench_cache_lookup_miss(c: &mut Criterion) {
    let cache = ExactCache::new(1000, Duration::from_secs(300));
    for i in 0..100u64 {
        cache.insert(i, make_response("cached response"));
    }

    c.bench_function("cache_lookup_miss", |b| {
        let mut miss_key = 100_000u64;
        b.iter(|| {
            miss_key += 1;
            black_box(cache.get(black_box(miss_key)))
        });
    });
}

fn bench_cache_ttl_expiry(c: &mut Criterion) {
    c.bench_function("cache_ttl_expiry_check", |b| {
        b.iter_batched(
            || {
                // Create cache with very short TTL and pre-populate
                let cache = ExactCache::new(1000, Duration::from_nanos(1));
                for i in 0..100u64 {
                    cache.insert(i, make_response("will expire"));
                }
                // All entries are expired by now (1ns TTL)
                cache
            },
            |cache| {
                // Lookup triggers expiry check and removal
                for i in 0..100u64 {
                    black_box(cache.get(black_box(i)));
                }
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_cache_lfu_eviction_under_pressure(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_lfu_eviction");

    for cap in [100, 500, 1_000] {
        group.bench_with_input(BenchmarkId::from_parameter(cap), &cap, |b, &cap| {
            b.iter_batched(
                || {
                    // Pre-fill cache to capacity with varying hit counts
                    let cache = ExactCache::new(cap, Duration::from_secs(300));
                    for i in 0..cap as u64 {
                        cache.insert(i, make_response("entry"));
                        // Give some entries higher hit counts
                        for _ in 0..(i % 10) {
                            cache.get(i);
                        }
                    }
                    cache
                },
                |cache| {
                    // Insert beyond capacity -- triggers LFU eviction each time
                    for i in cap as u64..(cap as u64 + 50) {
                        cache.insert(black_box(i), make_response("new entry"));
                    }
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn bench_cache_mixed_read_write(c: &mut Criterion) {
    c.bench_function("cache_mixed_read_write_ratio_80_20", |b| {
        b.iter_batched(
            || {
                let cache = ExactCache::new(5000, Duration::from_secs(300));
                for i in 0..1000u64 {
                    cache.insert(i, make_response("entry"));
                }
                cache
            },
            |cache| {
                // 80% reads, 20% writes -- simulates realistic access pattern
                for i in 0..1000u64 {
                    if i % 5 == 0 {
                        cache.insert(black_box(i + 10_000), make_response("new"));
                    } else {
                        black_box(cache.get(black_box(i % 1000)));
                    }
                }
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(
    benches,
    bench_cache_insert,
    bench_cache_lookup_hit,
    bench_cache_lookup_miss,
    bench_cache_ttl_expiry,
    bench_cache_lfu_eviction_under_pressure,
    bench_cache_mixed_read_write,
);
criterion_main!(benches);
