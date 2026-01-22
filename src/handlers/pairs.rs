use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::collections::HashMap;

use crate::{
    entities::{coins, prelude::Coins},
    models::{
        pairs::{TradeablePairsQuery, TradeablePairsResponse, TradeablePairInfo, AllTradeableAssetsResponse, TradeableAssetInfo, CoinMappingResponse, CoinMapping},
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
    match state.exchange_api.get_tradeable_tokens(symbols.clone()).await {
        Ok(tokens) => {
            // Fetch logos from coins table
            // We look up by symbol (uppercase) to match the coins table
            let logo_map = fetch_logos_for_symbols(&state, &symbols).await;

            // Map internal TradeableToken to public TradeablePairInfo
            let pairs: Vec<TradeablePairInfo> = tokens
                .into_iter()
                .map(|token| {
                    // Extract quote currency and build full trading pair
                    let quote_currency = token.trading_pair.to_uppercase(); // "USDC" or "USDT"
                    let trading_pair = format!("{}{}", token.symbol, quote_currency);

                    // Adjust priority based on prefer_usdc flag
                    let priority = adjust_priority(token.priority, prefer_usdc);

                    // Look up logo from the coins table (by symbol, case-insensitive)
                    let logo = logo_map.get(&token.symbol.to_uppercase()).cloned().flatten();

                    TradeablePairInfo {
                        coin_id: token.coin_id,
                        symbol: token.symbol,
                        exchange: token.exchange,
                        trading_pair,
                        quote_currency,
                        priority,
                        logo,
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

/// Coin info for logo lookup
struct CoinInfo {
    coin_id: String,
    logo_address: Option<String>,
}

/// Normalize an exchange symbol to match CoinGecko conventions
/// Examples: "1000SHIB" -> "shib", "1000FLOKI" -> "floki", "BTC" -> "btc"
fn normalize_symbol(symbol: &str) -> String {
    let s = symbol.to_lowercase();
    // Remove common prefixes used by exchanges
    let normalized = s
        .strip_prefix("1000")
        .or_else(|| s.strip_prefix("10000"))
        .or_else(|| s.strip_prefix("100000"))
        .unwrap_or(&s);
    normalized.to_string()
}

/// Fetch coin info (coin_id and logo) from coins table for given symbols
/// Returns a map of symbol (uppercase) -> CoinInfo
/// Uses both exact matching and normalized matching for better coverage
async fn fetch_coin_info_for_symbols(
    state: &AppState,
    symbols: &[String],
) -> HashMap<String, CoinInfo> {
    // Build list of symbols to query: both original and normalized
    let mut query_symbols: Vec<String> = Vec::new();
    let mut symbol_to_original: HashMap<String, String> = HashMap::new();

    for s in symbols {
        let upper = s.to_uppercase();
        let lower = s.to_lowercase();
        let normalized = normalize_symbol(s);

        // Add lowercase version
        if !query_symbols.contains(&lower) {
            query_symbols.push(lower.clone());
            symbol_to_original.insert(lower, upper.clone());
        }

        // Add normalized version if different
        if normalized != s.to_lowercase() && !query_symbols.contains(&normalized) {
            query_symbols.push(normalized.clone());
            symbol_to_original.insert(normalized, upper.clone());
        }
    }

    // Query coins table for all possible symbol variations
    let coins_result = Coins::find()
        .filter(coins::Column::Symbol.is_in(query_symbols))
        .all(&state.db)
        .await;

    match coins_result {
        Ok(coins_data) => {
            let mut coin_map: HashMap<String, CoinInfo> = HashMap::new();

            for coin in coins_data {
                let coin_symbol_lower = coin.symbol.to_lowercase();

                // Find which original symbol(s) this matches
                for (query_sym, original_sym) in &symbol_to_original {
                    if &coin_symbol_lower == query_sym {
                        // Don't overwrite if we already have a match (prefer exact match)
                        if !coin_map.contains_key(original_sym) {
                            coin_map.insert(original_sym.clone(), CoinInfo {
                                coin_id: coin.coin_id.clone(),
                                logo_address: coin.logo_address.clone(),
                            });
                        }
                    }
                }

                // Also store by the coin's own uppercase symbol for direct matches
                let coin_upper = coin.symbol.to_uppercase();
                if !coin_map.contains_key(&coin_upper) {
                    coin_map.insert(coin_upper, CoinInfo {
                        coin_id: coin.coin_id.clone(),
                        logo_address: coin.logo_address.clone(),
                    });
                }
            }

            tracing::debug!("Symbol matching found {} coin mappings for {} input symbols",
                coin_map.len(), symbols.len());
            coin_map
        }
        Err(e) => {
            tracing::warn!("Failed to fetch coin info from coins table: {}", e);
            HashMap::new()
        }
    }
}

/// Fetch logo URLs from coins table for given symbols (for tradeable pairs endpoint)
/// Returns a map of symbol (uppercase) -> Option<logo_address>
async fn fetch_logos_for_symbols(
    state: &AppState,
    symbols: &[String],
) -> HashMap<String, Option<String>> {
    let coin_map = fetch_coin_info_for_symbols(state, symbols).await;
    coin_map.into_iter().map(|(k, v)| (k, v.logo_address)).collect()
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

/// Handler for GET /api/exchange/all-tradeable-assets
/// Returns all unique tradeable symbols from Binance and Bitget
pub async fn get_all_tradeable_assets(
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<AllTradeableAssetsResponse>), (StatusCode, Json<ErrorResponse>)> {
    tracing::info!("Fetching all tradeable assets");

    match state.exchange_api.get_all_tradeable_symbols().await {
        Ok(tokens) => {
            // Collect all symbols to fetch coin info (coin_id and logos)
            let symbols: Vec<String> = tokens.iter().map(|t| t.symbol.clone()).collect();

            // Fetch coin info from coins table
            let coin_map = fetch_coin_info_for_symbols(&state, &symbols).await;

            // Map to response format
            let assets: Vec<TradeableAssetInfo> = tokens
                .into_iter()
                .map(|token| {
                    let quote_currency = token.trading_pair.to_uppercase();
                    let coin_info = coin_map.get(&token.symbol.to_uppercase());
                    let coin_id = coin_info.map(|c| c.coin_id.clone());
                    let logo = coin_info.and_then(|c| c.logo_address.clone());

                    TradeableAssetInfo {
                        symbol: token.symbol,
                        exchange: token.exchange,
                        quote_currency,
                        coin_id,
                        logo,
                    }
                })
                .collect();

            let total_count = assets.len();
            let cached = state.exchange_api.get_cache_age_secs().await > 0;

            tracing::info!("Returning {} tradeable assets", total_count);

            Ok((
                StatusCode::OK,
                Json(AllTradeableAssetsResponse {
                    assets,
                    total_count,
                    cached,
                }),
            ))
        }
        Err(e) => {
            tracing::error!("Failed to fetch all tradeable assets: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to fetch exchange data: {}", e),
                }),
            ))
        }
    }
}

/// Handler for GET /api/coins/symbol-mapping
/// Returns all coin symbol -> coin_id mappings from the database
pub async fn get_coin_symbol_mapping(
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<CoinMappingResponse>), (StatusCode, Json<ErrorResponse>)> {
    tracing::info!("Fetching all coin symbol mappings");

    // Query all active coins from the database
    let coins_result = Coins::find()
        .filter(coins::Column::Active.eq(true))
        .all(&state.db)
        .await;

    match coins_result {
        Ok(coins_data) => {
            let mappings: Vec<CoinMapping> = coins_data
                .into_iter()
                .map(|coin| CoinMapping {
                    symbol: coin.symbol.to_uppercase(),
                    coin_id: coin.coin_id,
                })
                .collect();

            let total_count = mappings.len();
            tracing::info!("Returning {} coin symbol mappings", total_count);

            Ok((
                StatusCode::OK,
                Json(CoinMappingResponse {
                    mappings,
                    total_count,
                }),
            ))
        }
        Err(e) => {
            tracing::error!("Failed to fetch coin mappings: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to fetch coin mappings: {}", e),
                }),
            ))
        }
    }
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
