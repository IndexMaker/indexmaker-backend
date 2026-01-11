use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexOrderbookResponse {
    pub index_id: i32,
    pub index_name: String,
    pub index_symbol: String,
    pub bids: Vec<OrderbookLevel>,
    pub asks: Vec<OrderbookLevel>,
    pub constituents: Vec<ConstituentOrderbook>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderbookLevel {
    pub price: f64,
    pub quantity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstituentOrderbook {
    pub coin_id: String,
    pub symbol: String,
    pub weight_percentage: f64,
    pub exchange: String,
    pub trading_pair: String,
}
