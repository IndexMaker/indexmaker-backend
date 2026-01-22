//! Virtual Orderbook Handler
//!
//! Provides aggregated orderbook preview for index compositions.
//! Used by the create-itp page to show liquidity depth before deployment.

use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::services::orderbook_aggregator::{
    AggregateOrderbookRequest, AggregatedOrderbook, OrderbookAggregator,
};

/// Request for virtual orderbook
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualOrderbookRequest {
    /// Asset symbols (e.g., ["BTC", "ETH", "SOL"])
    pub symbols: Vec<String>,
    /// Weights in basis points (must sum to 10000 = 100%)
    pub weights: Vec<u32>,
    /// Number of orderbook levels to return (default: 10)
    #[serde(default = "default_levels")]
    pub levels: Option<usize>,
    /// Aggregation depth in basis points (default: 10 = 0.1%)
    /// Options: 1 (0.01%), 5 (0.05%), 10 (0.1%), 25 (0.25%), 50 (0.5%), 100 (1%)
    #[serde(default = "default_aggregation_bps")]
    pub aggregation_bps: Option<u32>,
    /// Override the mid price (e.g., for ITP initial/current price)
    pub base_mid_price: Option<f64>,
}

fn default_levels() -> Option<usize> {
    Some(10)
}

fn default_aggregation_bps() -> Option<u32> {
    Some(10) // Default 0.1%
}

/// Response for virtual orderbook
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualOrderbookResponse {
    /// Success flag
    pub success: bool,
    /// Aggregated orderbook data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<OrderbookData>,
    /// Error message if failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Orderbook data in the response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderbookData {
    /// Bid levels (buy orders, highest price first)
    pub bids: Vec<OrderbookLevelResponse>,
    /// Ask levels (sell orders, lowest price first)
    pub asks: Vec<OrderbookLevelResponse>,
    /// Weighted mid price of the index
    pub mid_price: f64,
    /// Spread in basis points
    pub spread_bps: f64,
    /// Total bid depth in USD
    pub total_bid_depth_usd: f64,
    /// Total ask depth in USD
    pub total_ask_depth_usd: f64,
    /// Number of assets successfully fetched
    pub assets_included: usize,
    /// Total number of assets requested
    pub assets_requested: usize,
    /// Assets that failed to fetch
    pub assets_failed: Vec<String>,
    /// Per-asset liquidity breakdown
    pub asset_liquidity: Vec<AssetLiquidity>,
}

/// Single orderbook level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderbookLevelResponse {
    /// Price level
    pub price: f64,
    /// Quantity at this level
    pub quantity: f64,
    /// USD value (price * quantity)
    pub usd_value: f64,
    /// Cumulative USD depth from best price to this level
    pub cumulative_usd: f64,
}

/// Per-asset liquidity info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetLiquidity {
    /// Asset symbol
    pub symbol: String,
    /// Weight in basis points
    pub weight_bps: u32,
    /// Current mid price
    pub mid_price: f64,
    /// Spread in basis points
    pub spread_bps: f64,
    /// Bid side depth in USD
    pub bid_depth_usd: f64,
    /// Ask side depth in USD
    pub ask_depth_usd: f64,
}

/// POST /api/orderbook/virtual
///
/// Get a virtual aggregated orderbook for an index composition.
/// This allows previewing the liquidity depth before deploying an ITP.
///
/// Request body:
/// ```json
/// {
///   "symbols": ["BTC", "ETH", "SOL"],
///   "weights": [5000, 3000, 2000],
///   "levels": 10
/// }
/// ```
///
/// Response:
/// - Aggregated orderbook with bid/ask levels
/// - Total depth in USD
/// - Per-asset liquidity breakdown
pub async fn get_virtual_orderbook(
    State(state): State<crate::AppState>,
    Json(request): Json<VirtualOrderbookRequest>,
) -> impl IntoResponse {
    info!(
        "Virtual orderbook request: {} assets",
        request.symbols.len()
    );

    // Validate request
    if request.symbols.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(VirtualOrderbookResponse {
                success: false,
                data: None,
                error: Some("symbols cannot be empty".to_string()),
            }),
        );
    }

    if request.symbols.len() != request.weights.len() {
        return (
            StatusCode::BAD_REQUEST,
            Json(VirtualOrderbookResponse {
                success: false,
                data: None,
                error: Some("symbols and weights must have same length".to_string()),
            }),
        );
    }

    let total_weight: u32 = request.weights.iter().sum();
    if total_weight != 10000 {
        return (
            StatusCode::BAD_REQUEST,
            Json(VirtualOrderbookResponse {
                success: false,
                data: None,
                error: Some(format!(
                    "weights must sum to 10000 (100%), got {}",
                    total_weight
                )),
            }),
        );
    }

    // Check for invalid weights
    if request.weights.iter().any(|&w| w == 0) {
        return (
            StatusCode::BAD_REQUEST,
            Json(VirtualOrderbookResponse {
                success: false,
                data: None,
                error: Some("all weights must be > 0".to_string()),
            }),
        );
    }

    let levels = request.levels.unwrap_or(10).min(50); // Max 50 levels
    let aggregation_bps = request.aggregation_bps.unwrap_or(10); // Default 0.1%
    let base_mid_price = request.base_mid_price;
    let assets_requested = request.symbols.len();

    // Use live orderbook cache for real-time data
    let cache = &state.live_orderbook_cache;
    let aggregated = cache.get_aggregated(&request.symbols, &request.weights, levels, aggregation_bps, base_mid_price);

    // Convert to response format with cumulative depth
    let bids = add_cumulative_depth_from_cache(&aggregated.bids);
    let asks = add_cumulative_depth_from_cache(&aggregated.asks);

    // Get per-asset liquidity details from cache
    let asset_liquidity: Vec<AssetLiquidity> = request.symbols.iter().zip(request.weights.iter())
        .filter_map(|(symbol, weight)| {
            let key = if symbol.ends_with("USDT") {
                symbol.clone()
            } else {
                format!("{}USDT", symbol.to_uppercase())
            };
            cache.get_orderbook(&key).map(|ob| AssetLiquidity {
                symbol: symbol.clone(),
                weight_bps: *weight,
                mid_price: ob.mid_price,
                spread_bps: ob.spread_bps,
                bid_depth_usd: ob.bid_depth_usd(levels),
                ask_depth_usd: ob.ask_depth_usd(levels),
            })
        })
        .collect();

    info!(
        "Virtual orderbook from cache: mid=${:.2}, spread={:.1}bps, bid_depth=${:.0}, ask_depth=${:.0}, agg_bps={}",
        aggregated.mid_price,
        aggregated.spread_bps,
        aggregated.total_bid_depth_usd,
        aggregated.total_ask_depth_usd,
        aggregation_bps
    );

    (
        StatusCode::OK,
        Json(VirtualOrderbookResponse {
            success: true,
            data: Some(OrderbookData {
                bids,
                asks,
                mid_price: aggregated.mid_price,
                spread_bps: aggregated.spread_bps,
                total_bid_depth_usd: aggregated.total_bid_depth_usd,
                total_ask_depth_usd: aggregated.total_ask_depth_usd,
                assets_included: aggregated.assets_included,
                assets_requested,
                assets_failed: aggregated.assets_failed,
                asset_liquidity,
            }),
            error: None,
        }),
    )
}

/// Add cumulative USD depth to orderbook levels from cache
fn add_cumulative_depth_from_cache(
    levels: &[crate::services::live_orderbook_cache::OrderbookLevel],
) -> Vec<OrderbookLevelResponse> {
    let mut cumulative = 0.0;
    levels
        .iter()
        .map(|l| {
            cumulative += l.usd_value;
            OrderbookLevelResponse {
                price: l.price,
                quantity: l.quantity,
                usd_value: l.usd_value,
                cumulative_usd: cumulative,
            }
        })
        .collect()
}
