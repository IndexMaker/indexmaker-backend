use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepositTransactionSingle {
    pub index_id: i32,
    pub index_name: String,
    pub index_symbol: String,
    pub user: Option<String>,
    pub total_supply: String,
    pub total_quantity: String,
    pub supply_value_usd: f64,
    pub deposit_count: i32,
    pub supply: String,
    pub quantity: String,
    pub currency: String,
    pub share: f64,
    pub raw_share: f64,
    pub index_price: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepositTransactionAll {
    pub index_id: i32,
    pub name: String,
    pub symbol: String,
    pub address: String,
    pub user: Option<String>,
    pub total_supply: String,
    pub balance_raw: String,
    pub deposit_count: i32,
    pub supply: String,
    pub quantity: String,
    pub currency: String,
    pub share: f64,
    pub decimals: u32,
    pub share_pct: f64,
    pub usd_price: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DepositTransactionResponse {
    Single(Vec<DepositTransactionSingle>),
    All(Vec<DepositTransactionAll>),
}
