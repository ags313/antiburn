//! Canonical pricing value types.
//!
//! The pure data types the engine prices with: per-token model rates, a
//! per-model token breakdown, and the resulting cost. They carry no I/O and
//! no provider-specific dependencies.

use serde::{Deserialize, Serialize};

/// Model pricing rates (cost per token).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelPricing {
    pub input_cost_per_token: f64,
    pub output_cost_per_token: f64,
    pub cache_read_cost_per_token: f64,
    pub cache_write_cost_per_token: f64,
}

/// One model's token counts within a session or step.
///
/// Counts are non-negative (`u64`); callers whose source type is signed convert
/// at the boundary, saturating any negative value to zero.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelTokens {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    /// Cache-creation tokens explicitly classified as one-hour writes.
    ///
    /// This is a subset of `cache_creation_tokens`; the remainder is priced
    /// with the model's default cache-write rate.
    #[serde(default)]
    pub cache_creation_1h_tokens: u64,
    /// The cost that the agent recorded for these tokens, in millionths of a
    /// US dollar. The value is present only when every request in this record
    /// has a recorded cost. Cost calculation uses it instead of a catalog
    /// price. Resume snapshots use a binary encoding that cannot skip a
    /// field, so the field is always written.
    #[serde(default)]
    pub recorded_cost_micros: Option<u64>,
}

impl ModelTokens {
    /// Copy usage counts and the recorded cost into a pricing record.
    pub fn from_usage(usage: crate::analysis::Usage, recorded: Option<u64>) -> Self {
        Self {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            cache_creation_tokens: usage.cache_creation_tokens,
            cache_creation_1h_tokens: usage.cache_creation_1h_tokens,
            recorded_cost_micros: recorded,
        }
    }

    /// True when the record holds no tokens and no recorded cost.
    pub fn is_empty(&self) -> bool {
        self.input_tokens == 0
            && self.output_tokens == 0
            && self.cache_read_tokens == 0
            && self.cache_creation_tokens == 0
            && self.recorded_cost_micros.is_none()
    }

    /// Add the tokens of `other` to this record.
    ///
    /// The recorded cost stays only when each record that is not empty has
    /// one. A partial recorded cost must not replace the price of the other
    /// tokens.
    pub fn add(&mut self, other: &ModelTokens) {
        let recorded = if other.is_empty() {
            self.recorded_cost_micros
        } else if self.is_empty() {
            other.recorded_cost_micros
        } else {
            match (self.recorded_cost_micros, other.recorded_cost_micros) {
                (Some(left), Some(right)) => Some(left.saturating_add(right)),
                _ => None,
            }
        };
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(other.cache_read_tokens);
        self.cache_creation_tokens = self
            .cache_creation_tokens
            .saturating_add(other.cache_creation_tokens);
        self.cache_creation_1h_tokens = self
            .cache_creation_1h_tokens
            .saturating_add(other.cache_creation_1h_tokens);
        self.recorded_cost_micros = recorded;
    }
}

/// Estimated cost in USD, broken down by token category.
///
/// `total_usd` is the sum of the component fields; the breakdown is preserved
/// so UIs can render where the cost came from without recomputing it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Cost {
    pub total_usd: f64,
    pub input_usd: f64,
    pub output_usd: f64,
    pub cache_read_usd: f64,
    pub cache_write_usd: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(input: u64, recorded: Option<u64>) -> ModelTokens {
        ModelTokens {
            input_tokens: input,
            recorded_cost_micros: recorded,
            ..Default::default()
        }
    }

    #[test]
    fn from_usage_preserves_all_counts_and_optional_recorded_cost() {
        let usage = crate::analysis::Usage {
            input_tokens: 11,
            output_tokens: 22,
            cache_read_tokens: 33,
            cache_creation_tokens: 44,
            cache_creation_1h_tokens: 5,
        };
        for recorded in [None, Some(0), Some(u64::MAX)] {
            assert_eq!(
                ModelTokens::from_usage(usage, recorded),
                ModelTokens {
                    input_tokens: 11,
                    output_tokens: 22,
                    cache_read_tokens: 33,
                    cache_creation_tokens: 44,
                    cache_creation_1h_tokens: 5,
                    recorded_cost_micros: recorded,
                }
            );
        }
    }

    #[test]
    fn a_recorded_cost_survives_only_when_every_part_has_one() {
        let mut total = ModelTokens::default();
        total.add(&tokens(10, Some(5)));
        total.add(&tokens(20, Some(7)));
        assert_eq!(total.input_tokens, 30);
        assert_eq!(total.recorded_cost_micros, Some(12));

        // An empty record does not change the recorded cost.
        total.add(&ModelTokens::default());
        assert_eq!(total.recorded_cost_micros, Some(12));

        // Tokens without a recorded cost make the total unknown.
        total.add(&tokens(1, None));
        assert_eq!(total.recorded_cost_micros, None);
        total.add(&tokens(1, Some(1)));
        assert_eq!(total.recorded_cost_micros, None);

        let mut unknown_first = tokens(1, None);
        unknown_first.add(&tokens(1, Some(1)));
        assert_eq!(unknown_first.recorded_cost_micros, None);
    }
}
