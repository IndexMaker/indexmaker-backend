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

// NEW: Chart data entry structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartDataEntry {
    pub name: String,          // Index name
    pub date: DateTime<Utc>,   // ISO timestamp
    pub price: f64,            // Raw price from daily_prices
    pub value: f64,            // Cumulative value (starts at 10000)
}

// NEW: Updated response structure matching TypeScript
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexHistoricalDataResponse {
    pub name: String,                           // Index name
    pub index_id: i32,                          // Index ID
    pub chart_data: Vec<ChartDataEntry>,        // Historical data with cumulative returns
    pub formatted_transactions: Vec<String>,    // Empty for now
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexHistoricalDataQuery {
    pub start_date: Option<String>, // YYYY-MM-DD format
    pub end_date: Option<String>,   // YYYY-MM-DD format
}