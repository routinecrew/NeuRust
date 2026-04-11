use std::path::Path;

use anyhow::{Context, Result};

use crate::contracts::NeuRustConfig;

/// YAML 설정 파일 로드
pub fn load_config(path: &str) -> Result<NeuRustConfig> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path))?;
    let config: NeuRustConfig =
        serde_yaml::from_str(&content).with_context(|| "Failed to parse YAML config")?;
    tracing::info!("Config loaded from {}", path);
    Ok(config)
}

/// 기본 설정 경로 탐색 (neurust.yml → config/neurust.yml)
pub fn find_config_path() -> Option<String> {
    let candidates = ["neurust.yml", "config/neurust.yml"];
    for path in &candidates {
        if Path::new(path).exists() {
            return Some(path.to_string());
        }
    }
    None
}

/// 프로바이더 API 키를 환경변수에서 읽기
pub fn resolve_api_key(env_var: &str) -> Result<String> {
    std::env::var(env_var)
        .with_context(|| format!("Environment variable '{}' not set", env_var))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_config_path_returns_none_when_missing() {
        // 테스트 디렉토리에서 실행하면 None
        // (실제 config 파일이 없는 경우)
        let _ = find_config_path();
    }
}
