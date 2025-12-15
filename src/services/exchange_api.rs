use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::{Duration, SystemTime};

/// Tradeable token information from exchanges
#[derive(Debug, Clone)]
pub struct TradeableToken {
    pub coin_id: String,
    pub symbol: String,
    pub exchange: String,
    pub trading_pair: String,
    pub priority: u8, // Lower = higher priority (1=Binance USDC, 4=Bitget USDT)
}

/// Exchange API service for checking real-time tradeability
#[derive(Clone)]
pub struct ExchangeApiService {
    client: Client,
    cache: Arc<RwLock<ExchangeCache>>,
    cache_ttl_secs: u64,
}

struct ExchangeCache {
    binance_pairs: HashMap<String, Vec<String>>, // symbol -> [trading_pairs]
    bitget_pairs: HashMap<String, Vec<String>>,
    last_updated: SystemTime,
}

impl ExchangeCache {
    fn new() -> Self {
        Self {
            binance_pairs: HashMap::new(),
            bitget_pairs: HashMap::new(),
            last_updated: SystemTime::UNIX_EPOCH,
        }
    }

    fn is_expired(&self, ttl_secs: u64) -> bool {
        match self.last_updated.elapsed() {
            Ok(elapsed) => elapsed.as_secs() >= ttl_secs,
            Err(_) => true,
        }
    }
}

// Binance API response structures
#[derive(Debug, Deserialize)]
struct BinanceExchangeInfo {
    symbols: Vec<BinanceSymbol>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BinanceSymbol {
    symbol: String,
    base_asset: String,
    quote_asset: String,
    status: String,
}

// Bitget API response structures
#[derive(Debug, Deserialize)]
struct BitgetResponse {
    code: String,
    msg: String,
    data: Vec<BitgetSymbol>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BitgetSymbol {
    symbol: String,
    base_coin: String,
    quote_coin: String,
    status: String,
}

impl ExchangeApiService {
    pub fn new(cache_ttl_secs: u64) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap(),
            cache: Arc::new(RwLock::new(ExchangeCache::new())),
            cache_ttl_secs,
        }
    }

    /// Get tradeable tokens from exchanges for given symbols
    /// Returns tokens prioritized by: Binance USDC > Binance USDT > Bitget USDC > Bitget USDT
    pub async fn get_tradeable_tokens(
        &self,
        symbols: Vec<String>, // Uppercase symbols like ["BTC", "ETH", "SOL"]
    ) -> Result<Vec<TradeableToken>, Box<dyn std::error::Error + Send + Sync>> {
        // Refresh cache if expired
        {
            let cache = self.cache.read().await;
            if cache.is_expired(self.cache_ttl_secs) {
                drop(cache); // Release read lock
                self.refresh_cache().await?;
            }
        }

        // Read from cache
        let cache = self.cache.read().await;
        let mut tradeable = Vec::new();

        for symbol in symbols {
            let symbol_upper = symbol.to_uppercase();

            // Try to find best trading pair for this symbol
            if let Some(token) = self.find_best_pair(&cache, &symbol_upper) {
                tradeable.push(token);
            } else {
                tracing::debug!("Symbol {} not tradeable on any exchange", symbol_upper);
            }
        }

        Ok(tradeable)
    }

    /// Find best trading pair for a symbol with priority
    fn find_best_pair(&self, cache: &ExchangeCache, symbol: &str) -> Option<TradeableToken> {
        // Priority order: Binance USDC > Binance USDT > Bitget USDC > Bitget USDT
        
        // 1. Try Binance USDC
        if let Some(pairs) = cache.binance_pairs.get(symbol) {
            if pairs.contains(&"USDC".to_string()) {
                return Some(TradeableToken {
                    coin_id: symbol.to_lowercase(), // Will be resolved later
                    symbol: symbol.to_string(),
                    exchange: "binance".to_string(),
                    trading_pair: "usdc".to_string(),
                    priority: 1,
                });
            }
        }

        // 2. Try Binance USDT
        if let Some(pairs) = cache.binance_pairs.get(symbol) {
            if pairs.contains(&"USDT".to_string()) {
                return Some(TradeableToken {
                    coin_id: symbol.to_lowercase(),
                    symbol: symbol.to_string(),
                    exchange: "binance".to_string(),
                    trading_pair: "usdt".to_string(),
                    priority: 2,
                });
            }
        }

        // 3. Try Bitget USDC
        if let Some(pairs) = cache.bitget_pairs.get(symbol) {
            if pairs.contains(&"USDC".to_string()) {
                return Some(TradeableToken {
                    coin_id: symbol.to_lowercase(),
                    symbol: symbol.to_string(),
                    exchange: "bitget".to_string(),
                    trading_pair: "usdc".to_string(),
                    priority: 3,
                });
            }
        }

        // 4. Try Bitget USDT
        if let Some(pairs) = cache.bitget_pairs.get(symbol) {
            if pairs.contains(&"USDT".to_string()) {
                return Some(TradeableToken {
                    coin_id: symbol.to_lowercase(),
                    symbol: symbol.to_string(),
                    exchange: "bitget".to_string(),
                    trading_pair: "usdt".to_string(),
                    priority: 4,
                });
            }
        }

        None
    }

    /// Refresh cache by fetching from both exchanges
    async fn refresh_cache(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!("Refreshing exchange API cache...");

        // Fetch from both exchanges concurrently
        let binance_future = self.fetch_binance_pairs();
        let bitget_future = self.fetch_bitget_pairs();

        let (binance_result, bitget_result) = tokio::join!(binance_future, bitget_future);

        let binance_pairs = binance_result?;
        let bitget_pairs = bitget_result?;

        // Update cache
        let mut cache = self.cache.write().await;
        cache.binance_pairs = binance_pairs;
        cache.bitget_pairs = bitget_pairs;
        cache.last_updated = SystemTime::now();

        tracing::info!(
            "Exchange cache refreshed: {} Binance symbols, {} Bitget symbols",
            cache.binance_pairs.len(),
            cache.bitget_pairs.len()
        );

        Ok(())
    }

    /// Fetch Binance trading pairs
    async fn fetch_binance_pairs(
        &self,
    ) -> Result<HashMap<String, Vec<String>>, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://api.binance.com/api/v3/exchangeInfo";

        let response = self.fetch_with_retry(url, 3).await?;
        let exchange_info: BinanceExchangeInfo = response.json().await?;

        let mut pairs_map: HashMap<String, Vec<String>> = HashMap::new();

        for symbol_info in exchange_info.symbols {
            // Only include TRADING status
            if symbol_info.status != "TRADING" {
                continue;
            }

            // Only include USDC and USDT pairs
            if symbol_info.quote_asset != "USDC" && symbol_info.quote_asset != "USDT" {
                continue;
            }

            pairs_map
                .entry(symbol_info.base_asset)
                .or_insert_with(Vec::new)
                .push(symbol_info.quote_asset);
        }

        Ok(pairs_map)
    }

    /// Fetch Bitget trading pairs
    async fn fetch_bitget_pairs(
        &self,
    ) -> Result<HashMap<String, Vec<String>>, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://api.bitget.com/api/v2/spot/public/symbols";

        let response = self.fetch_with_retry(url, 3).await?;
        let bitget_response: BitgetResponse = response.json().await?;

        if bitget_response.code != "00000" {
            return Err(format!("Bitget API error: {}", bitget_response.msg).into());
        }

        let mut pairs_map: HashMap<String, Vec<String>> = HashMap::new();

        for symbol_info in bitget_response.data {
            // Only include online status
            if symbol_info.status != "online" {
                continue;
            }

            // Only include USDC and USDT pairs
            if symbol_info.quote_coin != "USDC" && symbol_info.quote_coin != "USDT" {
                continue;
            }

            pairs_map
                .entry(symbol_info.base_coin)
                .or_insert_with(Vec::new)
                .push(symbol_info.quote_coin);
        }

        Ok(pairs_map)
    }

    /// Fetch URL with exponential backoff retry
    async fn fetch_with_retry(
        &self,
        url: &str,
        max_retries: u32,
    ) -> Result<reqwest::Response, Box<dyn std::error::Error + Send + Sync>> {
        let mut delay = Duration::from_secs(1);

        for attempt in 0..max_retries {
            match self.client.get(url).send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        return Ok(response);
                    }

                    if attempt == max_retries - 1 {
                        return Err(format!("HTTP error: {}", response.status()).into());
                    }
                }
                Err(e) => {
                    if attempt == max_retries - 1 {
                        return Err(e.into());
                    }
                }
            }

            tracing::warn!(
                "Retry {}/{} for {}. Waiting {:?}",
                attempt + 1,
                max_retries,
                url,
                delay
            );

            tokio::time::sleep(delay).await;
            delay *= 2; // Exponential backoff
        }

        Err("Max retries exceeded".into())
    }

    /// Manual cache refresh (for testing or manual triggers)
    pub async fn force_refresh_cache(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.refresh_cache().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_tradeable_tokens() {
        let service = ExchangeApiService::new(600); // 10 min cache

        let symbols = vec![
            "BTC".to_string(),
            "ETH".to_string(),
            "SOL".to_string(),
            "INVALID_TOKEN_XYZ".to_string(),
        ];

        let result = service.get_tradeable_tokens(symbols).await;
        assert!(result.is_ok());

        let tradeable = result.unwrap();
        println!("Found {} tradeable tokens", tradeable.len());

        for token in &tradeable {
            println!(
                "  {} on {} with {} (priority: {})",
                token.symbol, token.exchange, token.trading_pair, token.priority
            );
        }

        // Should find BTC, ETH, SOL but not INVALID_TOKEN_XYZ
        assert!(tradeable.len() >= 3);
    }
}