//! Real-Time Price Service
//!
//! Continuously polls Binance and Bitget for all ticker prices every 5 seconds.
//! Stores prices in memory for fast access by ITP listing service.

use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Real-time price data
#[derive(Debug, Clone)]
pub struct PriceData {
    pub price: f64,
    pub exchange: String,
}

/// Real-time price service that polls exchanges continuously
#[derive(Clone)]
pub struct RealTimePriceService {
    client: Client,
    /// Symbol -> PriceData (e.g., "BTC" -> PriceData { price: 95000.0, exchange: "binance" })
    prices: Arc<RwLock<HashMap<String, PriceData>>>,
    poll_interval_secs: u64,
}

// Binance API response for ticker prices
#[derive(Debug, Deserialize)]
struct BinanceTickerPrice {
    symbol: String,
    price: String,
}

// Bitget API response for tickers
#[derive(Debug, Deserialize)]
struct BitgetTickerResponse {
    code: String,
    data: Vec<BitgetTicker>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BitgetTicker {
    symbol: String,
    last_pr: String, // Last price
}

impl RealTimePriceService {
    /// Create a new real-time price service
    pub fn new(poll_interval_secs: u64) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
            prices: Arc::new(RwLock::new(HashMap::new())),
            poll_interval_secs,
        }
    }

    /// Start the background polling task
    pub fn start_polling(&self) {
        let service = self.clone();
        tokio::spawn(async move {
            info!("Starting real-time price polling (every {} seconds)", service.poll_interval_secs);

            // Initial fetch
            if let Err(e) = service.fetch_all_prices().await {
                error!("Initial price fetch failed: {}", e);
            }

            let mut interval = tokio::time::interval(Duration::from_secs(service.poll_interval_secs));

            loop {
                interval.tick().await;

                if let Err(e) = service.fetch_all_prices().await {
                    error!("Price fetch failed: {}", e);
                }
            }
        });
    }

    /// Fetch all prices from both exchanges
    async fn fetch_all_prices(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Fetch from both exchanges concurrently
        let (binance_result, bitget_result) = tokio::join!(
            self.fetch_binance_prices(),
            self.fetch_bitget_prices()
        );

        let mut new_prices: HashMap<String, PriceData> = HashMap::new();

        // Process Bitget prices FIRST (primary exchange)
        match bitget_result {
            Ok(prices) => {
                let bitget_count = prices.len();
                for (symbol, price) in prices {
                    new_prices.insert(symbol, PriceData {
                        price,
                        exchange: "bitget".to_string(),
                    });
                }
                debug!("Fetched {} prices from Bitget", bitget_count);
            }
            Err(e) => {
                warn!("Failed to fetch Bitget prices: {}", e);
            }
        }

        // Process Binance prices (only add if not already from Bitget)
        match binance_result {
            Ok(prices) => {
                let mut binance_added = 0;
                for (symbol, price) in prices {
                    if !new_prices.contains_key(&symbol) {
                        new_prices.insert(symbol, PriceData {
                            price,
                            exchange: "binance".to_string(),
                        });
                        binance_added += 1;
                    }
                }
                debug!("Added {} additional prices from Binance", binance_added);
            }
            Err(e) => {
                warn!("Failed to fetch Binance prices: {}", e);
            }
        }

        // Update the cache
        {
            let mut cache = self.prices.write().await;
            *cache = new_prices;
        }

        let cache = self.prices.read().await;
        debug!("Total prices cached: {}", cache.len());

        Ok(())
    }

    /// Fetch all ticker prices from Binance
    async fn fetch_binance_prices(&self) -> Result<HashMap<String, f64>, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://api.binance.com/api/v3/ticker/price";

        let response = self.client.get(url).send().await?;

        if !response.status().is_success() {
            return Err(format!("Binance API error: {}", response.status()).into());
        }

        let tickers: Vec<BinanceTickerPrice> = response.json().await?;

        let mut prices: HashMap<String, f64> = HashMap::new();

        for ticker in tickers {
            // Parse the price
            let price: f64 = match ticker.price.parse() {
                Ok(p) => p,
                Err(_) => continue,
            };

            // Extract base symbol from trading pair (prefer USDT pairs for USD price)
            if let Some(base) = extract_base_symbol(&ticker.symbol, "USDT") {
                prices.insert(base, price);
            } else if let Some(base) = extract_base_symbol(&ticker.symbol, "USDC") {
                // Only insert USDC pair if we don't have USDT
                if !prices.contains_key(&base) {
                    prices.insert(base, price);
                }
            }
        }

        Ok(prices)
    }

    /// Fetch all ticker prices from Bitget
    async fn fetch_bitget_prices(&self) -> Result<HashMap<String, f64>, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://api.bitget.com/api/v2/spot/market/tickers";

        let response = self.client.get(url).send().await?;

        if !response.status().is_success() {
            return Err(format!("Bitget API error: {}", response.status()).into());
        }

        let bitget_response: BitgetTickerResponse = response.json().await?;

        if bitget_response.code != "00000" {
            return Err(format!("Bitget API error code: {}", bitget_response.code).into());
        }

        let mut prices: HashMap<String, f64> = HashMap::new();

        for ticker in bitget_response.data {
            // Parse the price
            let price: f64 = match ticker.last_pr.parse() {
                Ok(p) => p,
                Err(_) => continue,
            };

            // Bitget symbol format: BTCUSDT, ETHUSDC, etc.
            if let Some(base) = extract_base_symbol(&ticker.symbol, "USDT") {
                prices.insert(base, price);
            } else if let Some(base) = extract_base_symbol(&ticker.symbol, "USDC") {
                if !prices.contains_key(&base) {
                    prices.insert(base, price);
                }
            }
        }

        Ok(prices)
    }

    /// Get the current price for a symbol
    pub async fn get_price(&self, symbol: &str) -> Option<f64> {
        let cache = self.prices.read().await;
        cache.get(&symbol.to_uppercase()).map(|p| p.price)
    }

    /// Get all current prices
    pub async fn get_all_prices(&self) -> HashMap<String, f64> {
        let cache = self.prices.read().await;
        cache.iter().map(|(k, v)| (k.clone(), v.price)).collect()
    }

    /// Get price count (for health checks)
    pub async fn get_price_count(&self) -> usize {
        let cache = self.prices.read().await;
        cache.len()
    }
}

/// Extract base symbol from trading pair
/// e.g., "BTCUSDT" with quote "USDT" -> Some("BTC")
fn extract_base_symbol(pair: &str, quote: &str) -> Option<String> {
    let pair_upper = pair.to_uppercase();
    let quote_upper = quote.to_uppercase();

    if pair_upper.ends_with(&quote_upper) {
        let base = pair_upper.trim_end_matches(&quote_upper);
        if !base.is_empty() {
            return Some(base.to_string());
        }
    }
    None
}

impl Default for RealTimePriceService {
    fn default() -> Self {
        Self::new(5) // 5 second default
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_base_symbol() {
        assert_eq!(extract_base_symbol("BTCUSDT", "USDT"), Some("BTC".to_string()));
        assert_eq!(extract_base_symbol("ETHUSDC", "USDC"), Some("ETH".to_string()));
        assert_eq!(extract_base_symbol("SOLUSDT", "USDT"), Some("SOL".to_string()));
        assert_eq!(extract_base_symbol("BTCETH", "USDT"), None);
        assert_eq!(extract_base_symbol("USDT", "USDT"), None);
    }

    #[tokio::test]
    async fn test_fetch_prices() {
        let service = RealTimePriceService::new(5);

        // Test Binance fetch
        let binance_prices = service.fetch_binance_prices().await;
        assert!(binance_prices.is_ok());
        let prices = binance_prices.unwrap();
        assert!(prices.contains_key("BTC"));
        assert!(prices.contains_key("ETH"));
        println!("BTC price from Binance: {:?}", prices.get("BTC"));

        // Test Bitget fetch
        let bitget_prices = service.fetch_bitget_prices().await;
        assert!(bitget_prices.is_ok());
        let prices = bitget_prices.unwrap();
        println!("BTC price from Bitget: {:?}", prices.get("BTC"));
    }
}
