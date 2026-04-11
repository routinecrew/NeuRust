use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

use crate::contracts::*;
use crate::provider::ProviderRegistry;

/// 프로바이더 헬스 모니터 — 주기적으로 프로바이더 상태 확인
pub struct HealthMonitor {
    providers: Arc<ProviderRegistry>,
    statuses: Arc<RwLock<HashMap<String, ProviderHealth>>>,
    interval: Duration,
}

impl HealthMonitor {
    /// 새 헬스 모니터 생성
    pub fn new(providers: Arc<ProviderRegistry>, interval_secs: u64) -> Self {
        Self {
            providers,
            statuses: Arc::new(RwLock::new(HashMap::new())),
            interval: Duration::from_secs(interval_secs),
        }
    }

    /// 현재 프로바이더 상태 조회
    pub async fn get_status(&self, provider_name: &str) -> Option<ProviderHealth> {
        self.statuses.read().await.get(provider_name).cloned()
    }

    /// 전체 프로바이더 상태 조회
    pub async fn all_statuses(&self) -> HashMap<String, ProviderHealth> {
        self.statuses.read().await.clone()
    }

    /// 백그라운드 모니터링 루프 시작
    pub fn start(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(self.interval);
            loop {
                interval.tick().await;
                self.check_all().await;
            }
        })
    }

    async fn check_all(&self) {
        for provider in self.providers.all() {
            let health = provider.health_check().await;
            tracing::debug!(
                provider = %provider.name(),
                status = %health.status,
                latency_ms = health.latency_ms,
                "Health check"
            );
            self.statuses
                .write()
                .await
                .insert(provider.name().to_string(), health);
        }
    }
}
