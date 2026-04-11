use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tracing::info;

use neurust_core::config;
use neurust_core::contracts::{new_event_bus, Provider};
use neurust_core::provider::ProviderRegistry;
use neurust_gateway::server;
use neurust_gateway::state::AppState;
use neurust_intel::cache::exact_cache::ExactCache;
use neurust_intel::cache::warming::CacheWarmer;
use neurust_intel::cache::TieredCache;

#[tokio::main]
async fn main() -> Result<()> {
    // tracing 초기화
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("NeuRust — The Intelligent Rust Gateway");

    // 설정 로드
    let config_path = config::find_config_path()
        .unwrap_or_else(|| "config/neurust.yml".to_string());
    let cfg = config::load_config(&config_path)
        .with_context(|| format!("Failed to load config from {}", config_path))?;

    // 프로바이더 레지스트리 초기화
    let registry = ProviderRegistry::from_configs(&cfg.providers);
    let providers: Vec<Arc<dyn Provider>> = registry.all().to_vec();

    info!(
        providers = providers.len(),
        models = registry.all_models().len(),
        "Providers initialized"
    );

    // 이벤트 버스
    let (event_tx, _event_rx) = new_event_bus();

    // 캐시 초기화
    let cache = init_cache(&cfg).await;

    // 이벤트 스토어 초기화
    let event_store = init_event_store(&cfg);

    // 애플리케이션 상태 생성
    let mut state = AppState::new(providers, cfg, event_tx);
    if let Some(cache) = cache {
        state = state.with_cache(Arc::new(cache));
    }
    if let Some(store) = event_store {
        state = state.with_event_store(store);
    }

    // 서버 시작
    server::run(state).await
}

/// Initialize the tiered cache from configuration.
async fn init_cache(
    cfg: &neurust_core::contracts::NeuRustConfig,
) -> Option<TieredCache> {
    let intel_cfg = cfg.intelligence.as_ref()?;
    let cache_cfg = intel_cfg.tiered_cache.as_ref()?;
    let exact_cfg = cache_cfg.exact_cache.as_ref()?;

    if !exact_cfg.enabled {
        return None;
    }

    let max_entries = exact_cfg.max_entries.unwrap_or(100_000);
    let ttl_sec = exact_cfg.ttl_sec.unwrap_or(3600);
    let exact = Arc::new(ExactCache::new(max_entries, Duration::from_secs(ttl_sec)));

    // Cache warmer
    let warmer = exact_cfg
        .warming
        .as_ref()
        .filter(|w| w.enabled)
        .map(|w| {
            CacheWarmer::new(
                std::path::PathBuf::from(
                    w.dump_path
                        .clone()
                        .unwrap_or_else(|| "./cache_dump.bin".to_string()),
                ),
                Duration::from_secs(w.dump_interval_sec.unwrap_or(300)),
                w.max_warm_entries.unwrap_or(10_000),
            )
        });

    let tiered = TieredCache::new(Arc::clone(&exact), warmer);

    // Warm cache from dump
    match tiered.warm().await {
        Ok(count) if count > 0 => info!(entries = count, "Cache warmed from dump"),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "Cache warming failed (non-fatal)"),
    }

    info!(max_entries = max_entries, ttl_sec = ttl_sec, "Tiered cache initialized");
    Some(tiered)
}

/// Initialize the event store from configuration.
fn init_event_store(
    cfg: &neurust_core::contracts::NeuRustConfig,
) -> Option<Arc<dyn neurust_core::contracts::EventStore>> {
    let obs_cfg = cfg.observability.as_ref()?;
    let store_cfg = obs_cfg.store.as_ref()?;

    match store_cfg.store_type.as_deref() {
        #[cfg(feature = "store-sqlite")]
        Some("sqlite") => {
            let path = store_cfg
                .sqlite_path
                .as_deref()
                .unwrap_or("./neurust_events.db");
            match neurust_intel::store::sqlite_store::SqliteEventStore::open(path) {
                Ok(store) => {
                    info!(path = path, "SQLite event store initialized");
                    Some(Arc::new(store) as Arc<dyn neurust_core::contracts::EventStore>)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to open SQLite store, falling back to in-memory");
                    None
                }
            }
        }
        _ => None,
    }
}
