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
    pub token_ids: Vec<i32>,
    
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
