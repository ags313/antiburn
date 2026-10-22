//! Per-model cost calculation.

use std::collections::HashMap;

use crate::pricing::model::{Cost, ModelPricing, ModelTokens};
use crate::pricing::table::lookup_pricing;

/// The calculated cost and model IDs that have no price.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CostResult {
    pub cost: Cost,
    pub unpriced_models: Vec<String>,
}

/// Calculate cache-write cost, including the one-hour write premium.
pub fn calculate_cache_write_cost(tokens: &ModelTokens, pricing: &ModelPricing) -> f64 {
    let one_hour_tokens = tokens
        .cache_creation_1h_tokens
        .min(tokens.cache_creation_tokens);
    let default_tokens = tokens.cache_creation_tokens - one_hour_tokens;

    default_tokens as f64 * pricing.cache_write_cost_per_token
        + one_hour_tokens as f64 * pricing.input_cost_per_token * 2.0
}

/// Divide a recorded cost among the token categories by token count.
///
/// The agent records one total, not a cost for each category. The division
/// keeps the category values equal to the total.
pub fn split_recorded_cost(tokens: &ModelTokens, micros: u64) -> Cost {
    let total = micros as f64 / 1_000_000.0;
    let counts = [
        tokens.input_tokens,
        tokens.output_tokens,
        tokens.cache_read_tokens,
        tokens.cache_creation_tokens,
    ];
    let sum = counts
        .iter()
        .fold(0u64, |sum, count| sum.saturating_add(*count));
    if sum == 0 {
        return Cost {
            total_usd: total,
            input_usd: total,
            ..Cost::default()
        };
    }
    let share = |count: u64| total * count as f64 / sum as f64;
    let input_usd = share(counts[0]);
    let output_usd = share(counts[1]);
    let cache_read_usd = share(counts[2]);
    Cost {
        total_usd: total,
        input_usd,
        output_usd,
        cache_read_usd,
        cache_write_usd: total - input_usd - output_usd - cache_read_usd,
    }
}

/// Calculate the total estimated cost from a per-model token breakdown.
pub fn calculate_cost(
    breakdown: &HashMap<String, ModelTokens>,
    pricing_map: &HashMap<String, ModelPricing>,
) -> CostResult {
    let mut cost = Cost::default();
    let mut unpriced_models = Vec::new();
    let mut model_ids: Vec<&String> = breakdown.keys().collect();
    model_ids.sort_unstable();

    for model_id in model_ids {
        let tokens = &breakdown[model_id];
        if let Some(micros) = tokens.recorded_cost_micros {
            let recorded = split_recorded_cost(tokens, micros);
            cost.input_usd += recorded.input_usd;
            cost.output_usd += recorded.output_usd;
            cost.cache_read_usd += recorded.cache_read_usd;
            cost.cache_write_usd += recorded.cache_write_usd;
            cost.total_usd += recorded.total_usd;
            continue;
        }
        let Some(pricing) = lookup_pricing(model_id, pricing_map) else {
            unpriced_models.push(model_id.clone());
            continue;
        };

        let input = tokens.input_tokens as f64 * pricing.input_cost_per_token;
        let output = tokens.output_tokens as f64 * pricing.output_cost_per_token;
        let cache_read = tokens.cache_read_tokens as f64 * pricing.cache_read_cost_per_token;
        let cache_write = calculate_cache_write_cost(tokens, pricing);

        cost.input_usd += input;
        cost.output_usd += output;
        cost.cache_read_usd += cache_read;
        cost.cache_write_usd += cache_write;
        cost.total_usd += input + output + cache_read + cache_write;
    }

    CostResult {
        cost,
        unpriced_models,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recorded_cost_replaces_the_catalog_price_and_prices_unknown_models() {
        let tokens = |recorded| ModelTokens {
            input_tokens: 2,
            output_tokens: 1,
            cache_read_tokens: 1,
            recorded_cost_micros: recorded,
            ..Default::default()
        };
        let breakdown = HashMap::from([
            ("sample-model".to_string(), tokens(Some(0))),
            ("unknown-local-model".to_string(), tokens(Some(2_000_000))),
        ]);
        let result = calculate_cost(&breakdown, &pricing());
        assert!(result.unpriced_models.is_empty());
        assert!((result.cost.total_usd - 2.0).abs() < 1e-12);
        let parts = result.cost.input_usd
            + result.cost.output_usd
            + result.cost.cache_read_usd
            + result.cost.cache_write_usd;
        assert!((parts - result.cost.total_usd).abs() < 1e-12);
        assert!((result.cost.input_usd - 1.0).abs() < 1e-12);

        // Without a recorded cost, the unknown model stays unpriced.
        let breakdown = HashMap::from([("unknown-local-model".to_string(), tokens(None))]);
        assert_eq!(
            calculate_cost(&breakdown, &pricing()).unpriced_models,
            ["unknown-local-model"]
        );
    }

    fn pricing() -> HashMap<String, ModelPricing> {
        HashMap::from([(
            "sample-model".to_string(),
            ModelPricing {
                input_cost_per_token: 0.1,
                output_cost_per_token: 0.2,
                cache_read_cost_per_token: 0.03,
                cache_write_cost_per_token: 0.04,
            },
        )])
    }

    #[test]
    fn calculates_all_token_categories() {
        let tokens = ModelTokens {
            input_tokens: 2,
            output_tokens: 3,
            cache_read_tokens: 4,
            cache_creation_tokens: 5,
            cache_creation_1h_tokens: 0,
            recorded_cost_micros: None,
        };
        let result = calculate_cost(
            &HashMap::from([("sample-model".to_string(), tokens)]),
            &pricing(),
        );
        assert!((result.cost.total_usd - 1.12).abs() < 1e-12);
        assert!(result.unpriced_models.is_empty());
    }

    #[test]
    fn one_hour_writes_cost_twice_the_input_rate() {
        let tokens = ModelTokens {
            cache_creation_tokens: 5,
            cache_creation_1h_tokens: 2,
            ..Default::default()
        };
        let cost = calculate_cache_write_cost(&tokens, &pricing()["sample-model"]);
        assert!((cost - 0.52).abs() < 1e-12);
    }

    #[test]
    fn reports_unpriced_models_in_stable_order() {
        let breakdown = HashMap::from([
            ("z-model".to_string(), ModelTokens::default()),
            ("a-model".to_string(), ModelTokens::default()),
        ]);
        let result = calculate_cost(&breakdown, &HashMap::new());
        assert_eq!(result.unpriced_models, ["a-model", "z-model"]);
        assert_eq!(result.cost, Cost::default());
    }
}
