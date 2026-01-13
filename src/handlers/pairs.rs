use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};

use crate::{
    models::{
        pairs::{TradeablePairsQuery, TradeablePairsResponse, TradeablePairInfo},
        token::ErrorResponse,
    },
    AppState,
};

/// Handler for GET /api/exchange/tradeable-pairs
/// Fetches available trading pairs from Binance and Bitget exchanges
pub async fn get_tradeable_pairs(
    State(state): State<AppState>,
    Query(query): Query<TradeablePairsQuery>,
) -> Result<(StatusCode, Json<TradeablePairsResponse>), (StatusCode, Json<ErrorResponse>)> {
    tracing::info!("Fetching tradeable pairs with query: {:?}", query);

    // Parse coin_ids from comma-separated string
    let symbols: Vec<String> = if let Some(ref coin_ids_str) = query.coin_ids {
        coin_ids_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_uppercase())
            .collect()
    } else {
        // If no coin_ids provided, return all available pairs
        // For now, return common top coins
        vec![
            "BTC".to_string(),
            "ETH".to_string(),
            "SOL".to_string(),
            "BNB".to_string(),
            "XRP".to_string(),
        ]
    };

    // Get prefer_usdc flag (default: true)
    let prefer_usdc = query.prefer_usdc.unwrap_or(true);

    tracing::debug!(
        "Querying {} symbols with prefer_usdc={}",
        symbols.len(),
        prefer_usdc
    );

    // Call exchange API service to get tradeable tokens
    match state.exchange_api.get_tradeable_tokens(symbols).await {
        Ok(tokens) => {
            // Map internal TradeableToken to public TradeablePairInfo
            let pairs: Vec<TradeablePairInfo> = tokens
                .into_iter()
                .map(|token| {
                    // Extract quote currency and build full trading pair
                    let quote_currency = token.trading_pair.to_uppercase(); // "USDC" or "USDT"
                    let trading_pair = format!("{}{}", token.symbol, quote_currency);

                    // Adjust priority based on prefer_usdc flag
                    let priority = adjust_priority(token.priority, prefer_usdc);

                    TradeablePairInfo {
                        coin_id: token.coin_id,
                        symbol: token.symbol,
                        exchange: token.exchange,
                        trading_pair,
                        quote_currency,
                        priority,
                    }
                })
                .collect();

            // Sort by priority (lowest number = highest priority)
            let mut sorted_pairs = pairs;
            sorted_pairs.sort_by_key(|p| p.priority);

            // Get cache expiration time
            let cache_expires_in_secs = state.exchange_api.get_cache_age_secs().await;

            // Determine if this was a cache hit (cache_expires_in_secs > 0)
            let cached = cache_expires_in_secs > 0;

            // Build response
            let response = TradeablePairsResponse {
                pairs: sorted_pairs,
                cached,
                cache_expires_in_secs,
            };

            tracing::info!(
                "Successfully fetched {} tradeable pairs (cached: {}, expires_in: {}s)",
                response.pairs.len(),
                response.cached,
                response.cache_expires_in_secs
            );

            Ok((StatusCode::OK, Json(response)))
        }
        Err(e) => {
            tracing::error!("Failed to fetch tradeable pairs: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to fetch exchange data: {}", e),
                }),
            ))
        }
    }
}

/// Adjust priority based on prefer_usdc flag
/// If prefer_usdc=false, swap USDC/USDT priorities
fn adjust_priority(priority: u8, prefer_usdc: bool) -> u8 {
    if prefer_usdc {
        priority // Keep original priority (USDC preferred)
    } else {
        // Swap USDC/USDT priorities: 1↔2, 3↔4
        match priority {
            1 => 2, // Binance USDC → Binance USDT priority
            2 => 1, // Binance USDT → Binance USDC priority
            3 => 4, // Bitget USDC → Bitget USDT priority
            4 => 3, // Bitget USDT → Bitget USDC priority
            _ => priority,
        }
    }
}

/// Extract quote currency from trading pair
/// e.g., "BTCUSDC" → "USDC", "ETHUSDT" → "USDT"
#[allow(dead_code)]
fn extract_quote_currency(trading_pair: &str) -> String {
    // Take last 4 characters (USDC or USDT)
    trading_pair
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_quote_currency() {
        assert_eq!(extract_quote_currency("BTCUSDC"), "USDC");
        assert_eq!(extract_quote_currency("ETHUSDT"), "USDT");
        assert_eq!(extract_quote_currency("SOLUSDC"), "USDC");
        assert_eq!(extract_quote_currency("BNBUSDT"), "USDT");
    }

    #[test]
    fn test_adjust_priority_prefer_usdc_true() {
        // When prefer_usdc=true, priorities should remain unchanged
        assert_eq!(adjust_priority(1, true), 1); // Binance USDC
        assert_eq!(adjust_priority(2, true), 2); // Binance USDT
        assert_eq!(adjust_priority(3, true), 3); // Bitget USDC
        assert_eq!(adjust_priority(4, true), 4); // Bitget USDT
    }

    #[test]
    fn test_adjust_priority_prefer_usdc_false() {
        // When prefer_usdc=false, USDC/USDT priorities should be swapped
        assert_eq!(adjust_priority(1, false), 2); // Binance USDC → lower priority
        assert_eq!(adjust_priority(2, false), 1); // Binance USDT → higher priority
        assert_eq!(adjust_priority(3, false), 4); // Bitget USDC → lower priority
        assert_eq!(adjust_priority(4, false), 3); // Bitget USDT → higher priority
    }

    #[test]
    fn test_adjust_priority_edge_cases() {
        // Test edge cases (shouldn't happen, but good to handle)
        assert_eq!(adjust_priority(0, true), 0);
        assert_eq!(adjust_priority(5, true), 5);
        assert_eq!(adjust_priority(0, false), 0);
        assert_eq!(adjust_priority(5, false), 5);
    }
}
