use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalEntry {
    pub name: String,
    pub date: DateTime<Utc>,
    pub price: f64,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalDataResponse {
    pub data: Vec<HistoricalEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyPriceDataEntry {
    pub index: String,
    pub index_id: i32,
    pub date: String,
    pub quantities: HashMap<String, f64>,
    pub price: f64,
    pub value: f64,
    pub coin_prices: HashMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexHistoricalDataResponse {
    pub data: Vec<(i64, f64)>, // [[timestamp, price], ...]
}
