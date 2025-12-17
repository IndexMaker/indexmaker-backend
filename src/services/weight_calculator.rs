use rust_decimal::Decimal;
use std::collections::HashMap;

/// Weight calculation strategy
#[derive(Debug, Clone, PartialEq)]
pub enum WeightStrategy {
    Equal,
    MarketCap,
}

impl WeightStrategy {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "equal" => Some(WeightStrategy::Equal),
            "marketcap" => Some(WeightStrategy::MarketCap),
            _ => None,
        }
    }
}

/// Weight calculator for index constituents
pub struct WeightCalculator {
    strategy: WeightStrategy,
    threshold: Option<Decimal>, // Max weight percentage (e.g., 10.0 for 10%)
}

impl WeightCalculator {
    pub fn new(strategy: WeightStrategy, threshold: Option<Decimal>) -> Self {
        Self {
            strategy,
            threshold,
        }
    }

    /// Calculate weights for constituents based on strategy
    /// 
    /// Returns HashMap<coin_id, weight>
    pub fn calculate_weights(
        &self,
        coin_ids: &[String],
        market_caps: &HashMap<String, Decimal>,
        total_coins: usize,
    ) -> Result<HashMap<String, Decimal>, Box<dyn std::error::Error + Send + Sync>> {
        match self.strategy {
            WeightStrategy::Equal => self.calculate_equal_weights(coin_ids, total_coins),
            WeightStrategy::MarketCap => {
                self.calculate_market_cap_weights(coin_ids, market_caps, total_coins)
            }
        }
    }

    /// Calculate equal weights: weight = total_coins / num_constituents
    fn calculate_equal_weights(
        &self,
        coin_ids: &[String],
        total_coins: usize,
    ) -> Result<HashMap<String, Decimal>, Box<dyn std::error::Error + Send + Sync>> {
        if coin_ids.is_empty() {
            return Err("No constituents provided".into());
        }

        let weight = Decimal::from(total_coins) / Decimal::from(coin_ids.len());
        let mut weights = HashMap::new();

        for coin_id in coin_ids {
            weights.insert(coin_id.clone(), weight);
        }

        tracing::debug!(
            "Equal weight: {} coins, weight = {} each",
            coin_ids.len(),
            weight
        );

        Ok(weights)
    }

    /// Calculate market cap weighted weights with optional capping
    fn calculate_market_cap_weights(
        &self,
        coin_ids: &[String],
        market_caps: &HashMap<String, Decimal>,
        total_coins: usize,
    ) -> Result<HashMap<String, Decimal>, Box<dyn std::error::Error + Send + Sync>> {
        if coin_ids.is_empty() {
            return Err("No constituents provided".into());
        }

        // Filter out coins with missing or zero market cap
        let valid_coins: Vec<&String> = coin_ids
            .iter()
            .filter(|coin_id| {
                market_caps
                    .get(*coin_id)
                    .map(|mcap| *mcap > Decimal::ZERO)
                    .unwrap_or(false)
            })
            .collect();

        if valid_coins.is_empty() {
            return Err("No valid market cap data available for any constituent".into());
        }

        // Log excluded coins
        let excluded: Vec<&String> = coin_ids
            .iter()
            .filter(|coin_id| !valid_coins.contains(coin_id))
            .collect();

        if !excluded.is_empty() {
            tracing::warn!(
                "Excluding {} coins due to missing/zero market cap: {:?}",
                excluded.len(),
                excluded
            );
        }

        // Calculate total market cap
        let total_market_cap: Decimal = valid_coins
            .iter()
            .filter_map(|coin_id| market_caps.get(*coin_id))
            .sum();

        if total_market_cap == Decimal::ZERO {
            return Err("Total market cap is zero".into());
        }

        // Calculate raw weights (proportional to market cap)
        let mut weights = HashMap::new();
        let total_coins_decimal = Decimal::from(total_coins);

        for coin_id in &valid_coins {
            let mcap = market_caps.get(*coin_id).unwrap();
            let proportion = mcap / total_market_cap;
            let raw_weight = proportion * total_coins_decimal;

            // Apply cap if threshold is set
            let final_weight = if let Some(threshold) = self.threshold {
                if raw_weight > threshold {
                    tracing::debug!(
                        "Capping {} weight: {:.2} → {:.2}",
                        coin_id,
                        raw_weight,
                        threshold
                    );
                    threshold
                } else {
                    raw_weight
                }
            } else {
                raw_weight
            };

            weights.insert((*coin_id).clone(), final_weight);
        }

        // Log summary
        let total_weight: Decimal = weights.values().sum();
        tracing::debug!(
            "Market cap weights: {} coins, total weight = {:.2} (threshold: {:?})",
            weights.len(),
            total_weight,
            self.threshold
        );

        Ok(weights)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_equal_weights() {
        let calculator = WeightCalculator::new(WeightStrategy::Equal, None);
        let coin_ids = vec![
            "bitcoin".to_string(),
            "ethereum".to_string(),
            "solana".to_string(),
        ];

        let weights = calculator
            .calculate_weights(&coin_ids, &HashMap::new(), 100)
            .unwrap();

        assert_eq!(weights.len(), 3);
        assert_eq!(weights.get("bitcoin"), Some(&dec!(33.333333333333333333333333333)));
    }

    #[test]
    fn test_market_cap_weights_no_cap() {
        let calculator = WeightCalculator::new(WeightStrategy::MarketCap, None);
        let coin_ids = vec![
            "bitcoin".to_string(),
            "ethereum".to_string(),
            "solana".to_string(),
        ];

        let mut market_caps = HashMap::new();
        market_caps.insert("bitcoin".to_string(), dec!(1700000000000)); // 60% of total
        market_caps.insert("ethereum".to_string(), dec!(400000000000));  // 14.3%
        market_caps.insert("solana".to_string(), dec!(700000000000));    // 25%

        let weights = calculator
            .calculate_weights(&coin_ids, &market_caps, 100)
            .unwrap();

        // Bitcoin should have ~60% weight
        let btc_weight = weights.get("bitcoin").unwrap();
        assert!(*btc_weight > dec!(59) && *btc_weight < dec!(61));
    }

    #[test]
    fn test_market_cap_weights_with_cap() {
        let calculator = WeightCalculator::new(WeightStrategy::MarketCap, Some(dec!(10.0)));
        let coin_ids = vec![
            "bitcoin".to_string(),
            "ethereum".to_string(),
            "solana".to_string(),
        ];

        let mut market_caps = HashMap::new();
        market_caps.insert("bitcoin".to_string(), dec!(1700000000000)); // Would be 60%
        market_caps.insert("ethereum".to_string(), dec!(400000000000));  // Would be 14.3%
        market_caps.insert("solana".to_string(), dec!(700000000000));    // Would be 25%

        let weights = calculator
            .calculate_weights(&coin_ids, &market_caps, 100)
            .unwrap();

        // Bitcoin should be capped at 10.0
        assert_eq!(weights.get("bitcoin"), Some(&dec!(10.0)));
        
        // Ethereum should also be capped at 10.0
        assert_eq!(weights.get("ethereum"), Some(&dec!(10.0)));
    }

    #[test]
    fn test_market_cap_missing_data() {
        let calculator = WeightCalculator::new(WeightStrategy::MarketCap, None);
        let coin_ids = vec![
            "bitcoin".to_string(),
            "ethereum".to_string(),
            "missing_coin".to_string(),
        ];

        let mut market_caps = HashMap::new();
        market_caps.insert("bitcoin".to_string(), dec!(1000000000000));
        market_caps.insert("ethereum".to_string(), dec!(500000000000));
        // missing_coin has no market cap data

        let weights = calculator
            .calculate_weights(&coin_ids, &market_caps, 100)
            .unwrap();

        // Should only include bitcoin and ethereum
        assert_eq!(weights.len(), 2);
        assert!(weights.contains_key("bitcoin"));
        assert!(weights.contains_key("ethereum"));
        assert!(!weights.contains_key("missing_coin"));
    }

    #[test]
    fn test_equal_weights_empty_list() {
        let calculator = WeightCalculator::new(WeightStrategy::Equal, None);
        let coin_ids: Vec<String> = vec![];

        let result = calculator.calculate_weights(&coin_ids, &HashMap::new(), 100);
        assert!(result.is_err());
    }
}