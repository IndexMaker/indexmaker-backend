use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
