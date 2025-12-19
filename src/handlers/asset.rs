use axum::{extract::State, http::StatusCode, Json};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect};
use std::collections::{HashMap, HashSet};

use crate::entities::{daily_prices, rebalances, prelude::*};
use crate::models::asset::Asset;
use crate::models::token::ErrorResponse;
use crate::services::rebalancing::CoinRebalanceInfo;
use crate::AppState;

pub async fn fetch_all_assets(
    State(state): State<AppState>,
) -> Result<Json<Vec<Asset>>, (StatusCode, Json<ErrorResponse>)> {
    tracing::info!("Fetching all assets across indexes");

    // Step 1: Get all rebalances and extract unique index IDs
    let all_rebalances = Rebalances::find()
        .all(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error: {}", e),
                }),
            )
        })?;

    if all_rebalances.is_empty() {
        tracing::info!("No rebalances found");
        return Ok(Json(vec![]));
    }

    // Get unique index IDs
    let mut unique_indexes = HashSet::new();
    for rebalance in &all_rebalances {
        unique_indexes.insert(rebalance.index_id);
    }

    let index_ids: Vec<i32> = unique_indexes.into_iter().collect();

    tracing::info!("Found {} indexes with rebalances", index_ids.len());

    // Step 2: For each index, get latest rebalance and latest daily_price
    let mut all_coin_ids = HashSet::new();
    let mut expected_inventory: HashMap<String, f64> = HashMap::new();

    for index_id in index_ids {
        // Get latest rebalance for this index
        let latest_rebalance = Rebalances::find()
            .filter(rebalances::Column::IndexId.eq(index_id))
            .order_by(rebalances::Column::Timestamp, Order::Desc)
            .limit(1)
            .one(&state.db)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Database error: {}", e),
                    }),
                )
            })?;

        if latest_rebalance.is_none() {
            continue;
        }

        let rebalance = latest_rebalance.unwrap();

        // Parse coins from rebalance
        let coins: Vec<CoinRebalanceInfo> = serde_json::from_value(rebalance.coins.clone())
            .unwrap_or_default();

        // Get latest daily_price for this index
        let latest_daily_price = DailyPrices::find()
            .filter(daily_prices::Column::IndexId.eq(index_id.to_string()))
            .order_by(daily_prices::Column::Date, Order::Desc)
            .limit(1)
            .one(&state.db)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Database error: {}", e),
                    }),
                )
            })?;

        // Parse quantities from daily_price
        let quantities: HashMap<String, f64> = if let Some(daily) = latest_daily_price {
            if let Some(q) = daily.quantities {
                serde_json::from_value(q).unwrap_or_default()
            } else {
                HashMap::new()
            }
        } else {
            HashMap::new()
        };

        // Collect coin IDs and aggregate expected inventory
        for coin in coins {
            let coin_id = coin.coin_id.to_lowercase();
            all_coin_ids.insert(coin_id.clone());

            // Add quantity to expected inventory
            let qty = quantities.get(&coin.coin_id).copied().unwrap_or(0.0);
            *expected_inventory.entry(coin_id).or_insert(0.0) += qty;
        }
    }

    let coin_ids_vec: Vec<String> = all_coin_ids.into_iter().collect();
    tracing::info!(
        "Found {} unique coins across all indexes",
        coin_ids_vec.len()
    );

    if coin_ids_vec.is_empty() {
        return Ok(Json(vec![]));
    }

    // Step 3: Fetch market data from CoinGecko (chunked)
    let market_data = fetch_market_data_chunked(&state, &coin_ids_vec).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to fetch market data: {}", e),
            }),
        )
    })?;

    // Step 4: Build Asset[] response
    let mut assets: Vec<Asset> = market_data
        .into_iter()
        .filter_map(|coin| {
            let coin_id = coin.id.to_lowercase();
            let market_cap = coin.market_cap.unwrap_or(0.0);

            // Filter out coins with zero market cap
            if market_cap <= 0.0 {
                return None;
            }

            Some(Asset {
                id: coin.id.clone(),
                symbol: coin.symbol.to_uppercase(),
                name: coin.name,
                total_supply: coin.total_supply.unwrap_or(0.0),
                circulating_supply: coin.circulating_supply.unwrap_or(0.0),
                price_usd: coin.current_price.unwrap_or(0.0),
                market_cap,
                expected_inventory: expected_inventory.get(&coin_id).copied().unwrap_or(0.0),
                thumb: coin.image,
            })
        })
        .collect();

    // Step 5: Sort by market cap descending
    assets.sort_by(|a, b| {
        b.market_cap
            .partial_cmp(&a.market_cap)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    tracing::info!("Returning {} assets", assets.len());

    Ok(Json(assets))
}

// Keep fetch_market_data_chunked unchanged
async fn fetch_market_data_chunked(
    state: &AppState,
    coin_ids: &[String],
) -> Result<Vec<crate::models::asset::CoinGeckoMarketData>, Box<dyn std::error::Error + Send + Sync>>
{
    const CHUNK_SIZE: usize = 150;
    let mut all_market_data = Vec::new();

    // Split into chunks
    let chunks: Vec<&[String]> = coin_ids.chunks(CHUNK_SIZE).collect();
    
    tracing::info!(
        "Fetching market data in {} chunks ({} coins total)",
        chunks.len(),
        coin_ids.len()
    );

    for (i, chunk) in chunks.iter().enumerate() {
        tracing::debug!("Fetching chunk {}/{} ({} coins)", i + 1, chunks.len(), chunk.len());

        match state.coingecko.fetch_markets(chunk).await {
            Ok(data) => {
                all_market_data.extend(data);
            }
            Err(e) => {
                tracing::error!("Failed to fetch chunk {}: {}", i + 1, e);
                // Continue with other chunks instead of failing completely
            }
        }

        // Add delay between chunks to avoid rate limiting
        if i < chunks.len() - 1 {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
    }

    tracing::info!("Fetched market data for {} coins total", all_market_data.len());

    Ok(all_market_data)
}