use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexListEntry {
    pub index_id: i32,
    pub name: String,
    pub address: String,
    pub ticker: String,
    pub curator: String,
    pub total_supply: f64,
    #[serde(rename = "totalSupplyUSD")]
    pub total_supply_usd: f64,
    pub ytd_return: f64,
    pub collateral: Vec<CollateralToken>,
    pub management_fee: i32,  // Changed from f64 to i32
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inception_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratings: Option<Ratings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performance: Option<Performance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_price: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollateralToken {
    pub name: String,
    pub logo: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ratings {
    pub overall_rating: String,
    pub expense_rating: String,
    pub risk_rating: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Performance {
    pub ytd_return: f64,
    pub one_year_return: f64,
    pub three_year_return: f64,
    pub five_year_return: f64,
    pub ten_year_return: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexListResponse {
    pub indexes: Vec<IndexListEntry>,
}

impl Default for IndexListResponse {
    fn default() -> Self {
        Self {
            indexes: vec![],
        }
    }
}

impl Default for IndexListEntry {
    fn default() -> Self {
        Self {
            index_id: 0,
            name: String::new(),
            address: String::new(),
            ticker: String::new(),
            curator: String::new(),
            total_supply: 0.0,
            total_supply_usd: 0.0,
            ytd_return: 0.0,
            collateral: vec![],
            management_fee: 0,  // Changed from 0.0 to 0
            asset_class: None,
            inception_date: None,
            category: None,
            ratings: None,
            performance: None,
            index_price: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateIndexRequest {
    pub index_id: i32,
    pub name: String,
    pub symbol: String,
    pub address: String,
    pub category: Option<String>,
    pub asset_class: Option<String>,
    pub tokens: Vec<String>, // Array of token symbols
    pub top_x: Option<u32>,  // Top N coins by market cap

    // New rebalancing fields
    pub initial_date: NaiveDate,
    pub initial_price: Decimal,
    pub coingecko_category: String,
    pub exchanges_allowed: Vec<String>,
    pub exchange_trading_fees: Decimal,
    pub exchange_avg_spread: Decimal,
    pub rebalance_period: i32, // in days

    // Weight strategy fields (NEW)
    #[serde(default = "default_weight_strategy")]
    pub weight_strategy: String,  // "equal" or "marketCap"
    pub weight_threshold: Option<Decimal>,  // e.g., 10.0 for 10% cap

    #[serde(default)]
    pub blacklisted_categories: Option<Vec<String>>,
}

impl CreateIndexRequest {
    /// Validates that top_x is within the acceptable range (1-250)
    pub fn validate_top_x(&self) -> Result<(), String> {
        if let Some(top_x) = self.top_x {
            if top_x < 1 || top_x > 250 {
                return Err(format!("top_x must be between 1 and 250, got {}", top_x));
            }
        }
        Ok(())
    }

    /// Validates mutual exclusivity between top_x and tokens parameters
    pub fn validate_mutual_exclusivity(&self) -> Result<(), String> {
        let has_top_x = self.top_x.is_some();
        let has_tokens = !self.tokens.is_empty();

        if has_top_x && has_tokens {
            return Err("Cannot provide both top_x and tokens parameters".to_string());
        }

        if !has_top_x && !has_tokens {
            return Err("Must provide either top_x or tokens parameter".to_string());
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateIndexResponse {
    pub index_id: i32,
    pub name: String,
    pub symbol: String,
    pub address: String,
    pub category: Option<String>,
    pub asset_class: Option<String>,
    pub top_x: Option<u32>,

    // New rebalancing fields
    pub initial_date: NaiveDate,
    pub initial_price: String,
    pub coingecko_category: String,
    pub exchanges_allowed: Vec<String>,
    pub exchange_trading_fees: String,
    pub exchange_avg_spread: String,
    pub rebalance_period: i32,

    // Weight strategy fields (NEW)
    pub weight_strategy: String,
    pub weight_threshold: Option<String>,

    pub blacklisted_categories: Option<Vec<String>>,
}

// Default value helper
fn default_weight_strategy() -> String {
    "equal".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexConfigResponse {
    pub index_id: i32,
    pub symbol: String,
    pub name: String,
    pub address: String,
    pub initial_date: NaiveDate,
    pub initial_price: String,
    pub exchanges_allowed: Vec<String>,
    pub exchange_trading_fees: String,
    pub exchange_avg_spread: String,
    pub rebalance_period: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexPriceAtDateRequest {
    pub date: String, // YYYY-MM-DD format
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexPriceAtDateResponse {
    pub index_id: i32,
    pub date: String,
    pub price: f64,
    pub constituents: Vec<ConstituentPriceInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexLastPriceResponse {
    pub index_id: i32,
    pub timestamp: i64,        // Unix timestamp of last rebalance
    pub last_price: f64,       // Current index price
    pub last_bid: Option<f64>, // Not implemented yet
    pub last_ask: Option<f64>, // Not implemented yet
    pub constituents: Vec<ConstituentPriceInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstituentPriceInfo {
    pub coin_id: String,
    pub symbol: String,
    pub quantity: String,
    pub weight: String,
    pub price: f64,
    pub value: f64, // weight × quantity × price
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveIndexRequest {
    pub index_id: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveIndexResponse {
    pub success: bool,
    pub message: String,
    pub index_id: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentIndexWeightResponse {
    pub index_id: i32,
    pub index_name: String,
    pub index_symbol: String,
    pub last_rebalance_date: String,
    pub portfolio_value: String,
    pub total_weight: String,
    pub constituents: Vec<ConstituentWeight>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstituentWeight {
    pub coin_id: String,
    pub symbol: String,
    pub weight: String,
    pub weight_percentage: f64,
    pub quantity: String,
    pub price: f64,
    pub value: f64,
    pub exchange: String,
    pub trading_pair: String,
}

/// Request model for creating a manual index without automatic backfill
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateIndexManualRequest {
    pub index_id: i32,
    pub name: String,
    pub symbol: String,
    pub address: String,
    pub category: Option<String>,
    pub asset_class: Option<String>,
    pub initial_date: NaiveDate,
    pub initial_price: Decimal,
    pub skip_backfill: bool,
}

impl CreateIndexManualRequest {
    /// Validates that skip_backfill is explicitly set to true
    pub fn validate_skip_backfill(&self) -> Result<(), String> {
        if !self.skip_backfill {
            return Err("skip_backfill must be true for manual index creation".to_string());
        }
        Ok(())
    }

    /// Validates that no auto-selection parameters are provided
    pub fn validate_no_auto_selection(&self) -> Result<(), String> {
        // This validation will be enforced by the absence of these fields in the struct
        // Additional validation can be added if needed for JSON deserialization edge cases
        Ok(())
    }

    /// Comprehensive validation for manual index creation
    pub fn validate(&self) -> Result<(), String> {
        self.validate_skip_backfill()?;
        self.validate_no_auto_selection()?;
        Ok(())
    }
}

/// Response model for manual index creation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateIndexManualResponse {
    pub index_id: i32,
    pub name: String,
    pub symbol: String,
    pub address: String,
    pub category: Option<String>,
    pub asset_class: Option<String>,
    pub initial_date: NaiveDate,
    pub initial_price: String,
    pub skip_backfill: bool,
    pub next_steps: String,
}

/// Coin data for manual rebalance (per-coin rebalance information)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RebalanceCoin {
    pub coin_id: String,
    pub symbol: String,
    pub weight: String,
    pub quantity: String,
    pub price: f64,
    pub exchange: String,
    pub trading_pair: String,
}

/// Request model for manually adding a rebalance to a manual index
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualRebalanceRequest {
    pub date: NaiveDate,
    pub coins: Vec<RebalanceCoin>,
    pub portfolio_value: Decimal,
    pub total_weight: f64,
}

impl ManualRebalanceRequest {
    /// Validates that total_weight is approximately 1.0 (within tolerance of ±0.01)
    pub fn validate_weight_sum(&self) -> Result<(), String> {
        const MIN_WEIGHT: f64 = 0.99;
        const MAX_WEIGHT: f64 = 1.01;

        if self.total_weight < MIN_WEIGHT || self.total_weight > MAX_WEIGHT {
            return Err(format!(
                "Total weight must be between {} and {} (got: {})",
                MIN_WEIGHT, MAX_WEIGHT, self.total_weight
            ));
        }
        Ok(())
    }

    /// Validates that the date is not in the future
    pub fn validate_date_not_future(&self) -> Result<(), String> {
        use chrono::Utc;
        let today = Utc::now().date_naive();

        if self.date > today {
            return Err(format!(
                "Date cannot be in the future (got: {}, today: {})",
                self.date, today
            ));
        }
        Ok(())
    }
}

/// Response model for manual rebalance creation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualRebalanceResponse {
    pub success: bool,
    pub index_id: i32,
    pub rebalance_id: i32,
    pub date: String,
    pub portfolio_value: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    // Helper function to create a minimal valid CreateIndexRequest
    fn create_test_request() -> CreateIndexRequest {
        CreateIndexRequest {
            index_id: 1,
            name: "Test Index".to_string(),
            symbol: "TEST".to_string(),
            address: "0x123".to_string(),
            category: None,
            asset_class: None,
            tokens: vec![],
            top_x: None,
            initial_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            initial_price: dec!(100.0),
            coingecko_category: "test-category".to_string(),
            exchanges_allowed: vec!["binance".to_string()],
            exchange_trading_fees: dec!(0.001),
            exchange_avg_spread: dec!(0.0005),
            rebalance_period: 30,
            weight_strategy: "equal".to_string(),
            weight_threshold: None,
            blacklisted_categories: None,
        }
    }

    #[test]
    fn test_top_x_validation_valid_range() {
        let mut req = create_test_request();
        req.top_x = Some(50);
        req.tokens = vec![];
        assert!(req.validate_top_x().is_ok());
    }

    #[test]
    fn test_top_x_validation_min_boundary() {
        let mut req = create_test_request();
        req.top_x = Some(1);
        req.tokens = vec![];
        assert!(req.validate_top_x().is_ok());
    }

    #[test]
    fn test_top_x_validation_max_boundary() {
        let mut req = create_test_request();
        req.top_x = Some(250);
        req.tokens = vec![];
        assert!(req.validate_top_x().is_ok());
    }

    #[test]
    fn test_top_x_validation_below_range() {
        let mut req = create_test_request();
        req.top_x = Some(0);
        req.tokens = vec![];
        let result = req.validate_top_x();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must be between 1 and 250"));
    }

    #[test]
    fn test_top_x_validation_above_range() {
        let mut req = create_test_request();
        req.top_x = Some(251);
        req.tokens = vec![];
        let result = req.validate_top_x();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must be between 1 and 250"));
    }

    #[test]
    fn test_top_x_validation_none() {
        let mut req = create_test_request();
        req.top_x = None;
        req.tokens = vec!["BTC".to_string()];
        // None is valid (no range check needed)
        assert!(req.validate_top_x().is_ok());
    }

    #[test]
    fn test_mutual_exclusivity_both_provided() {
        let mut req = create_test_request();
        req.top_x = Some(50);
        req.tokens = vec!["BTC".to_string()];
        let result = req.validate_mutual_exclusivity();
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Cannot provide both top_x and tokens parameters"
        );
    }

    #[test]
    fn test_mutual_exclusivity_neither_provided() {
        let mut req = create_test_request();
        req.top_x = None;
        req.tokens = vec![];
        let result = req.validate_mutual_exclusivity();
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Must provide either top_x or tokens parameter"
        );
    }

    #[test]
    fn test_mutual_exclusivity_only_top_x() {
        let mut req = create_test_request();
        req.top_x = Some(50);
        req.tokens = vec![];
        assert!(req.validate_mutual_exclusivity().is_ok());
    }

    #[test]
    fn test_mutual_exclusivity_only_tokens() {
        let mut req = create_test_request();
        req.top_x = None;
        req.tokens = vec!["BTC".to_string(), "ETH".to_string()];
        assert!(req.validate_mutual_exclusivity().is_ok());
    }

    #[test]
    fn test_mutual_exclusivity_only_tokens_single() {
        let mut req = create_test_request();
        req.top_x = None;
        req.tokens = vec!["BTC".to_string()];
        assert!(req.validate_mutual_exclusivity().is_ok());
    }

    // Helper function to create a minimal valid CreateIndexManualRequest
    fn create_manual_test_request() -> CreateIndexManualRequest {
        CreateIndexManualRequest {
            index_id: 1,
            name: "Test Manual Index".to_string(),
            symbol: "MANUAL".to_string(),
            address: "0x456".to_string(),
            category: Some("DeFi".to_string()),
            asset_class: Some("Crypto".to_string()),
            initial_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            initial_price: dec!(100.0),
            skip_backfill: true,
        }
    }

    #[test]
    fn test_manual_request_skip_backfill_must_be_true() {
        let mut req = create_manual_test_request();
        req.skip_backfill = true;
        assert!(req.validate_skip_backfill().is_ok());
    }

    #[test]
    fn test_manual_request_skip_backfill_false_fails() {
        let mut req = create_manual_test_request();
        req.skip_backfill = false;
        let result = req.validate_skip_backfill();
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "skip_backfill must be true for manual index creation"
        );
    }

    #[test]
    fn test_manual_request_validation_passes() {
        let req = create_manual_test_request();
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_manual_request_validation_fails_on_skip_backfill() {
        let mut req = create_manual_test_request();
        req.skip_backfill = false;
        let result = req.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("skip_backfill must be true"));
    }

    #[test]
    fn test_manual_request_serialization() {
        let req = create_manual_test_request();
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"skipBackfill\":true"));
        assert!(json.contains("\"indexId\":1"));
    }

    #[test]
    fn test_manual_response_serialization() {
        let response = CreateIndexManualResponse {
            index_id: 1,
            name: "Test Manual Index".to_string(),
            symbol: "MANUAL".to_string(),
            address: "0x456".to_string(),
            category: Some("DeFi".to_string()),
            asset_class: Some("Crypto".to_string()),
            initial_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            initial_price: "100.0".to_string(),
            skip_backfill: true,
            next_steps: "Use POST /api/index/{index_id}/rebalance to manually add rebalances".to_string(),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"skipBackfill\":true"));
        assert!(json.contains("\"nextSteps\":"));
    }

    // Manual Rebalance Request Tests

    fn create_test_rebalance_request() -> ManualRebalanceRequest {
        ManualRebalanceRequest {
            date: NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
            coins: vec![
                RebalanceCoin {
                    coin_id: "bitcoin".to_string(),
                    symbol: "BTC".to_string(),
                    weight: "0.5".to_string(),
                    quantity: "0.01".to_string(),
                    price: 50000.0,
                    exchange: "binance".to_string(),
                    trading_pair: "BTC/USDT".to_string(),
                },
                RebalanceCoin {
                    coin_id: "ethereum".to_string(),
                    symbol: "ETH".to_string(),
                    weight: "0.5".to_string(),
                    quantity: "0.2".to_string(),
                    price: 2500.0,
                    exchange: "binance".to_string(),
                    trading_pair: "ETH/USDT".to_string(),
                },
            ],
            portfolio_value: dec!(1000.0),
            total_weight: 1.0,
        }
    }

    #[test]
    fn test_weight_sum_validation_exactly_one() {
        let request = create_test_rebalance_request();
        assert!(request.validate_weight_sum().is_ok());
    }

    #[test]
    fn test_weight_sum_validation_within_lower_tolerance() {
        let mut request = create_test_rebalance_request();
        request.total_weight = 0.99;
        assert!(request.validate_weight_sum().is_ok());
    }

    #[test]
    fn test_weight_sum_validation_within_upper_tolerance() {
        let mut request = create_test_rebalance_request();
        request.total_weight = 1.01;
        assert!(request.validate_weight_sum().is_ok());
    }

    #[test]
    fn test_weight_sum_validation_too_low() {
        let mut request = create_test_rebalance_request();
        request.total_weight = 0.98;
        let result = request.validate_weight_sum();
        assert!(result.is_err());
        let error_msg = result.unwrap_err();
        assert!(error_msg.contains("Total weight must be between"));
    }

    #[test]
    fn test_weight_sum_validation_too_high() {
        let mut request = create_test_rebalance_request();
        request.total_weight = 1.02;
        let result = request.validate_weight_sum();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Total weight must be between"));
    }

    #[test]
    fn test_weight_sum_validation_way_too_low() {
        let mut request = create_test_rebalance_request();
        request.total_weight = 0.5;
        let result = request.validate_weight_sum();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("0.5"));
    }

    #[test]
    fn test_date_validation_past_date_passes() {
        let mut request = create_test_rebalance_request();
        request.date = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
        assert!(request.validate_date_not_future().is_ok());
    }

    #[test]
    fn test_date_validation_today_passes() {
        let mut request = create_test_rebalance_request();
        request.date = chrono::Utc::now().date_naive();
        assert!(request.validate_date_not_future().is_ok());
    }

    #[test]
    fn test_date_validation_future_date_fails() {
        let mut request = create_test_rebalance_request();
        // Set date to tomorrow
        request.date = chrono::Utc::now().date_naive() + chrono::Duration::days(1);
        let result = request.validate_date_not_future();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Date cannot be in the future"));
    }

    #[test]
    fn test_rebalance_request_serialization() {
        let request = create_test_rebalance_request();
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"date\":\"2024-06-15\""));
        assert!(json.contains("\"portfolioValue\":\"1000.0\""));
        assert!(json.contains("\"totalWeight\":1.0"));
        assert!(json.contains("\"coinId\":\"bitcoin\""));
    }

    #[test]
    fn test_rebalance_coin_serialization() {
        let coin = RebalanceCoin {
            coin_id: "bitcoin".to_string(),
            symbol: "BTC".to_string(),
            weight: "0.5".to_string(),
            quantity: "0.01".to_string(),
            price: 50000.0,
            exchange: "binance".to_string(),
            trading_pair: "BTC/USDT".to_string(),
        };
        let json = serde_json::to_string(&coin).unwrap();
        assert!(json.contains("\"coinId\":\"bitcoin\""));
        assert!(json.contains("\"tradingPair\":\"BTC/USDT\""));
    }

    #[test]
    fn test_rebalance_response_serialization() {
        let response = ManualRebalanceResponse {
            success: true,
            index_id: 1,
            rebalance_id: 42,
            date: "2024-06-15".to_string(),
            portfolio_value: "1000.0".to_string(),
            message: "Manual rebalance added successfully".to_string(),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"rebalanceId\":42"));
        assert!(json.contains("\"portfolioValue\":\"1000.0\""));
    }
}
