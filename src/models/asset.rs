use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub id: String,
    pub symbol: String,
    pub name: String,
    pub total_supply: f64,
    pub circulating_supply: f64,
    pub price_usd: f64,
    pub market_cap: f64,
    pub expected_inventory: f64,
    pub thumb: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchAllAssetsResponse {
    pub assets: Vec<Asset>,
}

// CoinGecko market data response structure
#[derive(Debug, Clone, Deserialize)]
pub struct CoinGeckoMarketData {
    pub id: String,
    pub symbol: String,
    pub name: String,
    pub image: String,
    pub current_price: Option<f64>,
    pub market_cap: Option<f64>,
    pub market_cap_rank: Option<i32>,
    pub fully_diluted_valuation: Option<f64>,
    pub total_volume: Option<f64>,
    pub circulating_supply: Option<f64>,
    pub total_supply: Option<f64>,
    pub max_supply: Option<f64>,
}