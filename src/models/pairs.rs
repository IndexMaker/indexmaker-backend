use serde::{Deserialize, Serialize};

/// Query parameters for GET /api/exchange/tradeable-pairs
#[derive(Debug, Clone, Deserialize)]
pub struct TradeablePairsQuery {
    pub coin_ids: Option<String>,      // Comma-separated: "bitcoin,ethereum,solana"
    pub prefer_usdc: Option<bool>,     // Default: true
}

/// Single trading pair information in response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeablePairInfo {
    pub coin_id: String,
    pub symbol: String,
    pub exchange: String,              // "binance" or "bitget"
    pub trading_pair: String,          // "BTCUSDC", "ETHUSDT"
    pub quote_currency: String,        // "USDC" or "USDT"
    pub priority: u8,                  // 1-4
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,          // Logo URL from CoinGecko
}

/// Response structure for tradeable pairs endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeablePairsResponse {
    pub pairs: Vec<TradeablePairInfo>,
    pub cached: bool,
    pub cache_expires_in_secs: u64,
}

/// Single tradeable asset information (simplified for asset selection)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeableAssetInfo {
    pub symbol: String,
    pub exchange: String,              // "binance" or "bitget"
    pub quote_currency: String,        // "USDC" or "USDT"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coin_id: Option<String>,       // CoinGecko coin_id for logo lookup
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,          // Logo URL from CoinGecko
}

/// Response structure for all tradeable assets endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllTradeableAssetsResponse {
    pub assets: Vec<TradeableAssetInfo>,
    pub total_count: usize,
    pub cached: bool,
}

/// Single coin mapping entry (symbol -> coin_id)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoinMapping {
    pub symbol: String,
    pub coin_id: String,
}

/// Response structure for coin symbol mapping endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoinMappingResponse {
    pub mappings: Vec<CoinMapping>,
    pub total_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_coin_ids_single() {
        // Test parsing single coin_id
        let ids = "bitcoin";
        let parsed: Vec<String> = ids
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        assert_eq!(parsed, vec!["bitcoin"]);
    }

    #[test]
    fn test_parse_coin_ids_multiple() {
        // Test parsing multiple coin_ids
        let ids = "bitcoin,ethereum,solana";
        let parsed: Vec<String> = ids
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        assert_eq!(parsed, vec!["bitcoin", "ethereum", "solana"]);
    }

    #[test]
    fn test_parse_coin_ids_with_spaces() {
        // Test parsing with extra spaces
        let ids = " bitcoin , ethereum , solana ";
        let parsed: Vec<String> = ids
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        assert_eq!(parsed, vec!["bitcoin", "ethereum", "solana"]);
    }

    #[test]
    fn test_parse_empty_coin_ids() {
        // Test parsing empty strings
        let ids = "  ,  ,  ";
        let parsed: Vec<String> = ids
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        assert_eq!(parsed, Vec::<String>::new());
    }

    #[test]
    fn test_prefer_usdc_default() {
        // Test that prefer_usdc defaults to true
        let query = TradeablePairsQuery {
            coin_ids: None,
            prefer_usdc: None,
        };
        assert_eq!(query.prefer_usdc.unwrap_or(true), true);
    }

    #[test]
    fn test_prefer_usdc_false() {
        // Test explicit prefer_usdc=false
        let query = TradeablePairsQuery {
            coin_ids: None,
            prefer_usdc: Some(false),
        };
        assert_eq!(query.prefer_usdc.unwrap(), false);
    }

    #[test]
    fn test_serde_tradeable_pair_info() {
        // Test serialization of TradeablePairInfo
        let pair = TradeablePairInfo {
            coin_id: "bitcoin".to_string(),
            symbol: "BTC".to_string(),
            exchange: "binance".to_string(),
            trading_pair: "BTCUSDC".to_string(),
            quote_currency: "USDC".to_string(),
            priority: 1,
            logo: Some("https://coin-images.coingecko.com/coins/images/1/thumb/bitcoin.png".to_string()),
        };

        let json = serde_json::to_string(&pair).unwrap();
        assert!(json.contains("bitcoin"));
        assert!(json.contains("BTC"));
        assert!(json.contains("binance"));
        assert!(json.contains("BTCUSDC"));
        assert!(json.contains("USDC"));
        assert!(json.contains("logo"));

        // Test deserialization
        let deserialized: TradeablePairInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.coin_id, "bitcoin");
        assert_eq!(deserialized.symbol, "BTC");
        assert_eq!(deserialized.priority, 1);
        assert!(deserialized.logo.is_some());
    }

    #[test]
    fn test_serde_tradeable_pair_info_no_logo() {
        // Test that logo is skipped when None
        let pair = TradeablePairInfo {
            coin_id: "bitcoin".to_string(),
            symbol: "BTC".to_string(),
            exchange: "binance".to_string(),
            trading_pair: "BTCUSDC".to_string(),
            quote_currency: "USDC".to_string(),
            priority: 1,
            logo: None,
        };

        let json = serde_json::to_string(&pair).unwrap();
        // Logo should be omitted when None due to skip_serializing_if
        assert!(!json.contains("logo"));
    }

    #[test]
    fn test_serde_tradeable_pairs_response() {
        // Test serialization of response
        let response = TradeablePairsResponse {
            pairs: vec![
                TradeablePairInfo {
                    coin_id: "bitcoin".to_string(),
                    symbol: "BTC".to_string(),
                    exchange: "binance".to_string(),
                    trading_pair: "BTCUSDC".to_string(),
                    quote_currency: "USDC".to_string(),
                    priority: 1,
                    logo: None,
                },
            ],
            cached: true,
            cache_expires_in_secs: 600,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("pairs"));
        assert!(json.contains("cached"));
        assert!(json.contains("cache_expires_in_secs"));
        assert!(json.contains("600"));
    }
}
