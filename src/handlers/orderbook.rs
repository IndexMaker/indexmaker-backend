use axum::extract::{Path, State};
use axum::{http::StatusCode, Json};
use sea_orm::{ColumnTrait, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect};

use crate::entities::{index_metadata, prelude::*, rebalances};
use crate::models::orderbook::{
    ConstituentOrderbook, IndexOrderbookResponse, OrderbookLevel,
};
use crate::models::token::ErrorResponse;
use crate::services::orderbook::OrderbookService;
use crate::services::rebalancing::CoinRebalanceInfo;
use crate::AppState;

/// GET /indexes/{index_id}/orderbook
/// Returns the aggregated orderbook for an index based on constituent weights
pub async fn get_index_orderbook(
    State(state): State<AppState>,
    Path(index_id): Path<i32>,
) -> Result<Json<IndexOrderbookResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Get index metadata
    let index = IndexMetadata::find_by_id(index_id)
        .one(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error: {}", e),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Index {} not found", index_id),
                }),
            )
        })?;

    // Get last rebalance to get current constituent weights
    let last_rebalance = Rebalances::find()
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
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!(
                        "No rebalances found for index {} ({}). Index may not be initialized yet.",
                        index_id, index.name
                    ),
                }),
            )
        })?;

    // Parse constituents from last rebalance
    let coins: Vec<CoinRebalanceInfo> = serde_json::from_value(last_rebalance.coins.clone())
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to parse rebalance data: {}", e),
                }),
            )
        })?;

    // Calculate total weight for percentage calculations
    let total_weight: f64 = last_rebalance
        .total_weight
        .to_string()
        .parse()
        .unwrap_or(0.0);

    if total_weight == 0.0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Index has zero total weight".to_string(),
            }),
        ));
    }

    // Fetch orderbooks for each constituent
    let orderbook_service = OrderbookService::new();
    let mut orderbooks_with_weights = Vec::new();
    let mut constituent_info = Vec::new();

    for coin in coins {
        let weight: f64 = coin.weight.parse().unwrap_or(0.0);
        let weight_percentage = (weight / total_weight) * 100.0;

        // Fetch orderbook from exchange
        let orderbook_result = orderbook_service
            .fetch_orderbook(&coin.exchange, &coin.symbol, &coin.trading_pair, 100)
            .await;

        match orderbook_result {
            Ok(orderbook) => {
                tracing::debug!(
                    "Fetched orderbook for {} ({}%): {} bids, {} asks",
                    coin.symbol,
                    weight_percentage,
                    orderbook.bids.len(),
                    orderbook.asks.len()
                );
                orderbooks_with_weights.push((orderbook, weight_percentage));
                
                constituent_info.push(ConstituentOrderbook {
                    coin_id: coin.coin_id,
                    symbol: coin.symbol.clone(),
                    weight_percentage,
                    exchange: coin.exchange,
                    trading_pair: coin.trading_pair,
                });
            }
            Err(e) => {
                tracing::error!(
                    "Failed to fetch orderbook for {} on {}: {}",
                    coin.symbol,
                    coin.exchange,
                    e
                );
                // Continue with other constituents instead of failing the entire request
            }
        }
    }

    if orderbooks_with_weights.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Failed to fetch any orderbooks for index constituents".to_string(),
            }),
        ));
    }

    // Aggregate orderbooks with weights
    let aggregated = crate::services::orderbook::aggregate_weighted_orderbook(orderbooks_with_weights);

    // Convert to response format
    let bids: Vec<OrderbookLevel> = aggregated
        .bids
        .into_iter()
        .map(|level| OrderbookLevel {
            price: level.price,
            quantity: level.quantity,
        })
        .collect();

    let asks: Vec<OrderbookLevel> = aggregated
        .asks
        .into_iter()
        .map(|level| OrderbookLevel {
            price: level.price,
            quantity: level.quantity,
        })
        .collect();

    Ok(Json(IndexOrderbookResponse {
        index_id,
        index_name: index.name,
        index_symbol: index.symbol,
        bids,
        asks,
        constituents: constituent_info,
    }))
}
