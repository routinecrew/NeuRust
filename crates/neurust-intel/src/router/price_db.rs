//! # Model Pricing Database
//!
//! Model pricing database loaded from JSON.
//! Provides input/output token costs per model for cost-aware routing.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use serde::Deserialize;

/// Per-model pricing information.
#[derive(Clone, Debug, Deserialize)]
pub struct ModelPrice {
    /// Cost per 1K input tokens (USD).
    pub input_per_1k: f64,
    /// Cost per 1K output tokens (USD).
    pub output_per_1k: f64,
    /// Provider identifier (e.g., "openai", "anthropic").
    pub provider: String,
}

/// Raw JSON structure matching `data/model_prices.json`.
#[derive(Deserialize)]
struct PriceFile {
    models: HashMap<String, RawModelEntry>,
}

/// Raw per-model entry as stored in the JSON file.
#[derive(Deserialize)]
struct RawModelEntry {
    provider: String,
    input_per_1m_tokens: f64,
    output_per_1m_tokens: f64,
    #[allow(dead_code)]
    context_window: Option<u64>,
}

/// In-memory model pricing database.
///
/// Loaded from a JSON file or populated with hardcoded fallback prices
/// for common models.
#[derive(Clone, Debug)]
pub struct PriceDb {
    prices: HashMap<String, ModelPrice>,
}

impl PriceDb {
    /// Load pricing data from a JSON file.
    ///
    /// The file must follow the `data/model_prices.json` schema:
    /// `{ "models": { "<model_name>": { "provider", "input_per_1m_tokens", "output_per_1m_tokens", ... } } }`
    pub fn load(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(Path::new(path))
            .map_err(|e| anyhow::anyhow!("failed to read price file {}: {}", path, e))?;
        let file: PriceFile = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("failed to parse price file: {}", e))?;

        let prices = file
            .models
            .into_iter()
            .map(|(name, entry)| {
                let price = ModelPrice {
                    input_per_1k: entry.input_per_1m_tokens / 1000.0,
                    output_per_1k: entry.output_per_1m_tokens / 1000.0,
                    provider: entry.provider,
                };
                (name, price)
            })
            .collect();

        Ok(Self { prices })
    }

    /// Create a PriceDb with hardcoded prices for common models.
    ///
    /// Use this as a fallback when the JSON file is unavailable.
    pub fn load_embedded() -> Self {
        let mut prices = HashMap::new();

        // OpenAI
        prices.insert(
            "gpt-4o".to_string(),
            ModelPrice {
                input_per_1k: 0.0025,
                output_per_1k: 0.01,
                provider: "openai".to_string(),
            },
        );
        prices.insert(
            "gpt-4o-mini".to_string(),
            ModelPrice {
                input_per_1k: 0.00015,
                output_per_1k: 0.0006,
                provider: "openai".to_string(),
            },
        );
        prices.insert(
            "gpt-4.1".to_string(),
            ModelPrice {
                input_per_1k: 0.002,
                output_per_1k: 0.008,
                provider: "openai".to_string(),
            },
        );
        prices.insert(
            "gpt-4.1-mini".to_string(),
            ModelPrice {
                input_per_1k: 0.0004,
                output_per_1k: 0.0016,
                provider: "openai".to_string(),
            },
        );
        prices.insert(
            "gpt-4.1-nano".to_string(),
            ModelPrice {
                input_per_1k: 0.0001,
                output_per_1k: 0.0004,
                provider: "openai".to_string(),
            },
        );

        // Anthropic
        prices.insert(
            "claude-opus-4-6".to_string(),
            ModelPrice {
                input_per_1k: 0.015,
                output_per_1k: 0.075,
                provider: "anthropic".to_string(),
            },
        );
        prices.insert(
            "claude-sonnet-4-20250514".to_string(),
            ModelPrice {
                input_per_1k: 0.003,
                output_per_1k: 0.015,
                provider: "anthropic".to_string(),
            },
        );
        prices.insert(
            "claude-haiku-4-5-20251001".to_string(),
            ModelPrice {
                input_per_1k: 0.0008,
                output_per_1k: 0.004,
                provider: "anthropic".to_string(),
            },
        );

        Self { prices }
    }

    /// Look up pricing for a model by name.
    pub fn get(&self, model: &str) -> Option<&ModelPrice> {
        self.prices.get(model)
    }

    /// Estimate the cost (USD) for a request given token counts.
    ///
    /// Returns `None` if the model is not found in the database.
    pub fn estimate_cost(
        &self,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Option<f64> {
        let price = self.prices.get(model)?;
        let input_cost = (input_tokens as f64 / 1000.0) * price.input_per_1k;
        let output_cost = (output_tokens as f64 / 1000.0) * price.output_per_1k;
        Some(input_cost + output_cost)
    }

    /// Number of models in the database.
    pub fn len(&self) -> usize {
        self.prices.len()
    }

    /// Whether the database is empty.
    pub fn is_empty(&self) -> bool {
        self.prices.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_embedded() {
        let db = PriceDb::load_embedded();
        assert!(!db.is_empty());
        assert!(db.get("gpt-4o").is_some());
        assert!(db.get("claude-sonnet-4-20250514").is_some());
    }

    #[test]
    fn test_get_unknown_model() {
        let db = PriceDb::load_embedded();
        assert!(db.get("nonexistent-model").is_none());
    }

    #[test]
    fn test_estimate_cost() {
        let db = PriceDb::load_embedded();

        // gpt-4o: input=0.0025/1k, output=0.01/1k
        let cost = db.estimate_cost("gpt-4o", 1000, 500).unwrap();
        let expected = 0.0025 + (500.0 / 1000.0) * 0.01;
        assert!(
            (cost - expected).abs() < 1e-10,
            "cost={}, expected={}",
            cost,
            expected
        );
    }

    #[test]
    fn test_estimate_cost_unknown_model() {
        let db = PriceDb::load_embedded();
        assert!(db.estimate_cost("nonexistent", 100, 100).is_none());
    }

    #[test]
    fn test_estimate_cost_zero_tokens() {
        let db = PriceDb::load_embedded();
        let cost = db.estimate_cost("gpt-4o", 0, 0).unwrap();
        assert!((cost - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_load_from_file() {
        // Use the actual model_prices.json in the repo
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../data/model_prices.json"
        );
        if std::path::Path::new(path).exists() {
            let db = PriceDb::load(path).expect("should load price file");
            assert!(db.len() >= 5, "should have at least 5 models");

            // Verify conversion from per-1M to per-1K
            let gpt4o = db.get("gpt-4o").expect("gpt-4o should exist");
            assert!(
                (gpt4o.input_per_1k - 0.0025).abs() < 1e-10,
                "input_per_1k={}, expected 0.0025",
                gpt4o.input_per_1k
            );
        }
    }

    #[test]
    fn test_load_invalid_path() {
        let result = PriceDb::load("/nonexistent/path.json");
        assert!(result.is_err());
    }

    #[test]
    fn test_embedded_prices_match_known_values() {
        let db = PriceDb::load_embedded();

        // Verify a few known prices
        let opus = db.get("claude-opus-4-6").unwrap();
        assert_eq!(opus.provider, "anthropic");
        assert!((opus.input_per_1k - 0.015).abs() < 1e-10);
        assert!((opus.output_per_1k - 0.075).abs() < 1e-10);

        let mini = db.get("gpt-4o-mini").unwrap();
        assert_eq!(mini.provider, "openai");
        assert!((mini.input_per_1k - 0.00015).abs() < 1e-10);
    }
}
