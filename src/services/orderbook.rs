use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

/// Orderbook level (price, quantity)
#[derive(Debug, Clone)]
pub struct OrderbookLevel {
    pub price: f64,
    pub quantity: f64,
}

/// Complete orderbook for a trading pair
#[derive(Debug, Clone)]
pub struct Orderbook {
    pub symbol: String,
    pub exchange: String,
    pub bids: Vec<OrderbookLevel>, // Buy orders (highest price first)
    pub asks: Vec<OrderbookLevel>, // Sell orders (lowest price first)
}

/// Binance orderbook API response
#[derive(Debug, Deserialize)]
struct BinanceDepthResponse {
    bids: Vec<[String; 2]>, // [price, quantity]
    asks: Vec<[String; 2]>,
}

/// Bitget orderbook API response
#[derive(Debug, Deserialize)]
struct BitgetDepthResponse {
    code: String,
    msg: String,
    data: BitgetDepthData,
}

#[derive(Debug, Deserialize)]
struct BitgetDepthData {
    bids: Vec<[String; 2]>,
    asks: Vec<[String; 2]>,
}

/// Service for fetching orderbook data from exchanges
#[derive(Clone)]
pub struct OrderbookService {
    client: Client,
}

impl OrderbookService {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
        }
    }

    /// Fetch orderbook from Binance
    pub async fn fetch_binance_orderbook(
        &self,
        symbol: &str,
        trading_pair: &str,
        limit: usize,
    ) -> Result<Orderbook, Box<dyn std::error::Error + Send + Sync>> {
        let trading_symbol = format!("{}{}", symbol.to_uppercase(), trading_pair.to_uppercase());
        let url = format!(
            "https://api.binance.com/api/v3/depth?symbol={}&limit={}",
            trading_symbol,
            limit.min(5000) // Binance max is 5000
        );

        let response = self.client.get(&url).send().await?;
        
        if !response.status().is_success() {
            return Err(format!("Binance API error: {}", response.status()).into());
        }

        let depth: BinanceDepthResponse = response.json().await?;

        let bids = depth
            .bids
            .into_iter()
            .map(|level| OrderbookLevel {
                price: level[0].parse().unwrap_or(0.0),
                quantity: level[1].parse().unwrap_or(0.0),
            })
            .collect();

        let asks = depth
            .asks
            .into_iter()
            .map(|level| OrderbookLevel {
                price: level[0].parse().unwrap_or(0.0),
                quantity: level[1].parse().unwrap_or(0.0),
            })
            .collect();

        Ok(Orderbook {
            symbol: symbol.to_string(),
            exchange: "binance".to_string(),
            bids,
            asks,
        })
    }

    /// Fetch orderbook from Bitget
    pub async fn fetch_bitget_orderbook(
        &self,
        symbol: &str,
        trading_pair: &str,
        limit: usize,
    ) -> Result<Orderbook, Box<dyn std::error::Error + Send + Sync>> {
        let trading_symbol = format!("{}{}SPBL", symbol.to_uppercase(), trading_pair.to_uppercase());
        let url = format!(
            "https://api.bitget.com/api/v2/spot/market/orderbook?symbol={}&type=step0&limit={}",
            trading_symbol,
            limit.min(150) // Bitget max is 150
        );

        let response = self.client.get(&url).send().await?;
        
        if !response.status().is_success() {
            return Err(format!("Bitget API error: {}", response.status()).into());
        }

        let depth: BitgetDepthResponse = response.json().await?;

        if depth.code != "00000" {
            return Err(format!("Bitget API error: {}", depth.msg).into());
        }

        let bids = depth
            .data
            .bids
            .into_iter()
            .map(|level| OrderbookLevel {
                price: level[0].parse().unwrap_or(0.0),
                quantity: level[1].parse().unwrap_or(0.0),
            })
            .collect();

        let asks = depth
            .data
            .asks
            .into_iter()
            .map(|level| OrderbookLevel {
                price: level[0].parse().unwrap_or(0.0),
                quantity: level[1].parse().unwrap_or(0.0),
            })
            .collect();

        Ok(Orderbook {
            symbol: symbol.to_string(),
            exchange: "bitget".to_string(),
            bids,
            asks,
        })
    }

    /// Fetch orderbook from specified exchange
    pub async fn fetch_orderbook(
        &self,
        exchange: &str,
        symbol: &str,
        trading_pair: &str,
        limit: usize,
    ) -> Result<Orderbook, Box<dyn std::error::Error + Send + Sync>> {
        match exchange.to_lowercase().as_str() {
            "binance" => self.fetch_binance_orderbook(symbol, trading_pair, limit).await,
            "bitget" => self.fetch_bitget_orderbook(symbol, trading_pair, limit).await,
            _ => Err(format!("Unsupported exchange: {}", exchange).into()),
        }
    }
}

/// Weighted orderbook aggregation
/// Takes multiple orderbooks and their weights, returns a weighted aggregate
pub fn aggregate_weighted_orderbook(
    orderbooks: Vec<(Orderbook, f64)>, // (orderbook, weight_percentage)
) -> AggregatedOrderbook {
    let mut aggregated_bids: Vec<AggregatedOrderbookLevel> = Vec::new();
    let mut aggregated_asks: Vec<AggregatedOrderbookLevel> = Vec::new();

    // Process each orderbook with its weight
    for (orderbook, weight_percentage) in orderbooks {
        let weight_factor = weight_percentage / 100.0;

        // Aggregate bids
        for bid in orderbook.bids {
            aggregated_bids.push(AggregatedOrderbookLevel {
                price: bid.price,
                quantity: bid.quantity * weight_factor,
                coin_id: orderbook.symbol.clone(),
            });
        }

        // Aggregate asks
        for ask in orderbook.asks {
            aggregated_asks.push(AggregatedOrderbookLevel {
                price: ask.price,
                quantity: ask.quantity * weight_factor,
                coin_id: orderbook.symbol.clone(),
            });
        }
    }

    // Sort bids by price descending (highest price first)
    aggregated_bids.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap());

    // Sort asks by price ascending (lowest price first)
    aggregated_asks.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap());

    AggregatedOrderbook {
        bids: aggregated_bids,
        asks: aggregated_asks,
    }
}

#[derive(Debug, Clone)]
pub struct AggregatedOrderbook {
    pub bids: Vec<AggregatedOrderbookLevel>,
    pub asks: Vec<AggregatedOrderbookLevel>,
}

#[derive(Debug, Clone)]
pub struct AggregatedOrderbookLevel {
    pub price: f64,
    pub quantity: f64,
    pub coin_id: String,
}
