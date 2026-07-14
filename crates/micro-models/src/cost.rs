use serde::{Deserialize, Serialize};

/// Tokens consumed by one request, as reported by the provider.
///
/// `input` counts only tokens billed at the full input rate: providers report
/// cached tokens separately, and this type keeps them separate so each is
/// priced at its own rate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    #[serde(default)]
    pub input: u64,
    #[serde(default)]
    pub output: u64,
    #[serde(default)]
    pub cache_read: u64,
    #[serde(default)]
    pub cache_write: u64,
}

impl TokenUsage {
    pub fn new(input: u64, output: u64) -> Self {
        TokenUsage {
            input,
            output,
            ..Default::default()
        }
    }

    pub fn with_cache(mut self, read: u64, write: u64) -> Self {
        self.cache_read = read;
        self.cache_write = write;
        self
    }

    /// Every token the request touched, cached or not.
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

impl std::ops::Add for TokenUsage {
    type Output = TokenUsage;

    fn add(self, rhs: TokenUsage) -> TokenUsage {
        TokenUsage {
            input: self.input + rhs.input,
            output: self.output + rhs.output,
            cache_read: self.cache_read + rhs.cache_read,
            cache_write: self.cache_write + rhs.cache_write,
        }
    }
}

impl std::ops::AddAssign for TokenUsage {
    fn add_assign(&mut self, rhs: TokenUsage) {
        *self = *self + rhs;
    }
}

/// What a request cost, in US dollars, broken down by what was billed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct RequestCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

impl RequestCost {
    pub fn total(&self) -> f64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

impl std::ops::Add for RequestCost {
    type Output = RequestCost;

    fn add(self, rhs: RequestCost) -> RequestCost {
        RequestCost {
            input: self.input + rhs.input,
            output: self.output + rhs.output,
            cache_read: self.cache_read + rhs.cache_read,
            cache_write: self.cache_write + rhs.cache_write,
        }
    }
}

impl std::ops::AddAssign for RequestCost {
    fn add_assign(&mut self, rhs: RequestCost) {
        *self = *self + rhs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Catalog, ModelCost};

    /// Comparing dollar amounts, tolerant of binary floating-point rounding.
    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn prices_each_token_class_at_its_own_rate() {
        let cost = ModelCost {
            input: 5.0,
            output: 25.0,
            cache_read: 0.5,
            cache_write: 6.25,
        };

        let priced = cost.price(TokenUsage {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 1_000_000,
        });

        assert_close(priced.input, 5.0);
        assert_close(priced.output, 25.0);
        assert_close(priced.cache_read, 0.5);
        assert_close(priced.cache_write, 6.25);
        assert_close(priced.total(), 36.75);
    }

    #[test]
    fn prices_a_realistic_request() {
        let catalog = Catalog::bundled();
        let opus = catalog.get("anthropic", "claude-opus-5").unwrap();

        // 12k fresh input, 3k output, 40k served from cache.
        let priced = opus.price(TokenUsage::new(12_000, 3_000).with_cache(40_000, 0));

        assert_close(priced.input, 0.06);
        assert_close(priced.output, 0.075);
        assert_close(priced.cache_read, 0.02);
        assert_close(priced.total(), 0.155);
    }

    #[test]
    fn a_zero_priced_model_costs_nothing() {
        let cost = ModelCost::default();
        assert!(cost.is_free());
        assert_close(cost.price(TokenUsage::new(500_000, 200_000)).total(), 0.0);
    }

    #[test]
    fn empty_usage_costs_nothing() {
        let catalog = Catalog::bundled();
        let opus = catalog.get("anthropic", "claude-opus-5").unwrap();
        assert_close(opus.price(TokenUsage::default()).total(), 0.0);
    }

    #[test]
    fn sub_million_token_counts_scale_linearly() {
        let cost = ModelCost {
            input: 3.0,
            ..Default::default()
        };
        assert_close(cost.price(TokenUsage::new(1, 0)).input, 0.000_003);
        assert_close(cost.price(TokenUsage::new(333_333, 0)).input, 0.999_999);
    }

    #[test]
    fn usage_and_cost_accumulate_across_a_session() {
        let catalog = Catalog::bundled();
        let sonnet = catalog.get("anthropic", "claude-sonnet-5").unwrap();

        let mut usage = TokenUsage::default();
        let mut spent = RequestCost::default();
        for _ in 0..3 {
            let turn = TokenUsage::new(10_000, 1_000);
            usage += turn;
            spent += sonnet.price(turn);
        }

        assert_eq!(usage, TokenUsage::new(30_000, 3_000));
        assert_eq!(usage.total(), 33_000);
        // 30k input at $2/M plus 3k output at $10/M.
        assert_close(spent.total(), 0.06 + 0.03);
    }
}
