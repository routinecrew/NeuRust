use thiserror::Error;

/// NeuRust 도메인 에러
#[derive(Error, Debug)]
pub enum NeuRustError {
    #[error("Provider error: {0}")]
    Provider(String),
    #[error("Config error: {0}")]
    Config(String),
    #[error("Auth error: {0}")]
    Auth(String),
    #[error("Pipeline error: {0}")]
    Pipeline(String),
    #[error("No available provider for model: {0}")]
    NoProvider(String),
    #[error("Rate limited")]
    RateLimited,
}
