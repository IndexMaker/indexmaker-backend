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
pub struct AddIndexRequest {
    pub index_id: i32,
    pub name: String,
    pub symbol: String,
    pub address: String,
    pub category: Option<String>,
    pub asset_class: Option<String>,
    pub tokens: Vec<String>, // Array of token symbols
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddIndexResponse {
    pub index_id: i32,
    pub name: String,
    pub symbol: String,
    pub address: String,
    pub category: Option<String>,
    pub asset_class: Option<String>,
    pub token_ids: Vec<i32>,
}
