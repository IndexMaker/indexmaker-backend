use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, Serializer};
use std::collections::HashMap;


// Custom serializer to format dates with milliseconds (matches TypeScript format)
fn serialize_datetime_with_millis<S>(date: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let formatted = date.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    serializer.serialize_str(&formatted)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalEntry {
    pub name: String,
    #[serde(serialize_with = "serialize_datetime_with_millis")]
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
    #[serde(serialize_with = "serialize_datetime_with_millis")]
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

// Market cap historical data
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketCapEntry {
    pub coin_id: String,
    pub symbol: String,
    #[serde(serialize_with = "serialize_datetime_with_millis")]
    pub date: DateTime<Utc>,
    pub market_cap: f64,
    pub price: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketCapDataResponse {
    pub data: Vec<MarketCapEntry>,
}
