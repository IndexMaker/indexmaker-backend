//! WebSocket handler for live orderbook streaming
//!
//! Provides real-time aggregated orderbook updates for index compositions.
//! Uses the LiveOrderbookCache which is fed by Bitget WebSocket connections.

use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, State},
    response::IntoResponse,
    Json,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;
use tracing::{debug, error, info, warn};

use crate::AppState;
use crate::services::live_orderbook_cache::{LiveOrderbookCache, AggregatedLiveOrderbook, OrderbookLevel as CacheLevel};

/// GET /api/orderbook/cache-stats - Debug endpoint to check live cache state
pub async fn cache_stats(State(state): State<AppState>) -> Json<serde_json::Value> {
    let cache = &state.live_orderbook_cache;
    let symbols = cache.symbols();
    let sample: Vec<_> = symbols.iter().take(20).cloned().collect();

    Json(serde_json::json!({
        "total_symbols": symbols.len(),
        "sample_symbols": sample,
        "has_btc": symbols.contains(&"BTCUSDT".to_string()),
        "has_eth": symbols.contains(&"ETHUSDT".to_string()),
        "has_sol": symbols.contains(&"SOLUSDT".to_string()),
    }))
}

/// WebSocket subscription request from client
#[derive(Debug, Clone, Deserialize)]
pub struct WsSubscribeRequest {
    /// Action type
    pub action: String,
    /// Asset symbols (e.g., ["BTC", "ETH", "SOL"])
    pub symbols: Vec<String>,
    /// Weights in basis points (must sum to 10000)
    pub weights: Vec<u32>,
    /// Number of orderbook levels (default: 10)
    #[serde(default = "default_levels")]
    pub levels: usize,
    /// Update interval in milliseconds (default: 100, min: 50)
    #[serde(default = "default_interval")]
    pub interval_ms: u64,
    /// Aggregation depth in basis points (default: 10 = 0.1%)
    /// Lower values = more granular levels, higher values = more aggregated depth
    #[serde(default = "default_aggregation_bps")]
    pub aggregation_bps: u32,
    /// Override the mid price (e.g., for ITP initial/current price)
    /// If not provided, uses weighted average of underlying assets
    pub base_mid_price: Option<f64>,
}

fn default_aggregation_bps() -> u32 {
    10 // Default 0.1% aggregation
}

fn default_levels() -> usize {
    10
}

fn default_interval() -> u64 {
    100 // Default to 100ms for snappy updates
}

/// WebSocket message to client
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum WsMessage {
    /// Subscription confirmed
    #[serde(rename = "subscribed")]
    Subscribed {
        symbols: Vec<String>,
        weights: Vec<u32>,
        interval_ms: u64,
    },
    /// Orderbook update
    #[serde(rename = "orderbook")]
    Orderbook {
        data: OrderbookData,
        timestamp: u64,
    },
    /// Error message
    #[serde(rename = "error")]
    Error { message: String },
    /// Ping/pong for keepalive
    #[serde(rename = "pong")]
    Pong,
}

/// Orderbook data in WebSocket message
#[derive(Debug, Clone, Serialize)]
pub struct OrderbookData {
    pub bids: Vec<OrderbookLevel>,
    pub asks: Vec<OrderbookLevel>,
    pub mid_price: f64,
    pub spread_bps: f64,
    pub total_bid_depth_usd: f64,
    pub total_ask_depth_usd: f64,
    pub assets_included: usize,
    pub assets_failed: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrderbookLevel {
    pub price: f64,
    pub quantity: f64,
    pub usd_value: f64,
}

impl From<AggregatedLiveOrderbook> for OrderbookData {
    fn from(agg: AggregatedLiveOrderbook) -> Self {
        Self {
            bids: agg
                .bids
                .into_iter()
                .map(|l| OrderbookLevel {
                    price: l.price,
                    quantity: l.quantity,
                    usd_value: l.usd_value,
                })
                .collect(),
            asks: agg
                .asks
                .into_iter()
                .map(|l| OrderbookLevel {
                    price: l.price,
                    quantity: l.quantity,
                    usd_value: l.usd_value,
                })
                .collect(),
            mid_price: agg.mid_price,
            spread_bps: agg.spread_bps,
            total_bid_depth_usd: agg.total_bid_depth_usd,
            total_ask_depth_usd: agg.total_ask_depth_usd,
            assets_included: agg.assets_included,
            assets_failed: agg.assets_failed,
        }
    }
}

/// GET /api/orderbook/ws
///
/// WebSocket endpoint for live orderbook streaming.
/// Uses in-memory cache fed by Bitget WebSocket connections for real-time data.
///
/// Client sends subscription request:
/// ```json
/// {
///   "action": "subscribe",
///   "symbols": ["BTC", "ETH", "SOL"],
///   "weights": [5000, 3000, 2000],
///   "levels": 10,
///   "interval_ms": 100
/// }
/// ```
///
/// Server streams orderbook updates:
/// ```json
/// {
///   "type": "orderbook",
///   "data": { "bids": [...], "asks": [...], ... },
///   "timestamp": 1234567890
/// }
/// ```
pub async fn orderbook_websocket(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state.live_orderbook_cache))
}

async fn handle_socket(socket: WebSocket, cache: Arc<LiveOrderbookCache>) {
    let (mut sender, mut receiver) = socket.split();

    info!("New orderbook WebSocket connection (cache has {} symbols)", cache.symbol_count());

    // Wait for subscription request
    let subscription = match wait_for_subscription(&mut receiver).await {
        Ok(sub) => sub,
        Err(e) => {
            let _ = sender
                .send(Message::Text(
                    serde_json::to_string(&WsMessage::Error {
                        message: e.to_string(),
                    })
                    .unwrap().into(),
                ))
                .await;
            return;
        }
    };

    info!(
        "Orderbook subscription: {} symbols, {}ms interval",
        subscription.symbols.len(),
        subscription.interval_ms
    );

    // Send subscription confirmation
    let _ = sender
        .send(Message::Text(
            serde_json::to_string(&WsMessage::Subscribed {
                symbols: subscription.symbols.clone(),
                weights: subscription.weights.clone(),
                interval_ms: subscription.interval_ms,
            })
            .unwrap().into(),
        ))
        .await;

    // Prepare for streaming
    let symbols = subscription.symbols;
    let weights = subscription.weights;
    let levels = subscription.levels;
    let aggregation_bps = subscription.aggregation_bps;
    let base_mid_price = subscription.base_mid_price;

    // Start streaming updates - minimum 50ms interval for live feel
    let interval_duration = Duration::from_millis(subscription.interval_ms.max(50));
    let mut ticker = interval(interval_duration);

    // Track last sent data to avoid sending duplicates
    let mut last_mid_price: f64 = 0.0;

    loop {
        tokio::select! {
            // Send orderbook update on interval
            _ = ticker.tick() => {
                let orderbook = cache.get_aggregated(&symbols, &weights, levels, aggregation_bps, base_mid_price);

                // Skip if no real change (mid price same)
                if (orderbook.mid_price - last_mid_price).abs() < 0.000001 && last_mid_price > 0.0 {
                    continue;
                }
                last_mid_price = orderbook.mid_price;

                let msg = WsMessage::Orderbook {
                    data: orderbook.into(),
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                };

                if let Err(e) = sender.send(Message::Text(
                    serde_json::to_string(&msg).unwrap().into()
                )).await {
                    debug!("WebSocket send error: {}", e);
                    break;
                }
            }

            // Handle incoming messages (ping/pong, unsubscribe)
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(req) = serde_json::from_str::<serde_json::Value>(&text) {
                            if req.get("action").and_then(|a| a.as_str()) == Some("ping") {
                                let _ = sender.send(Message::Text(
                                    serde_json::to_string(&WsMessage::Pong).unwrap().into()
                                )).await;
                            } else if req.get("action").and_then(|a| a.as_str()) == Some("unsubscribe") {
                                info!("Client unsubscribed");
                                break;
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = sender.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!("WebSocket closed by client");
                        break;
                    }
                    Some(Err(e)) => {
                        error!("WebSocket receive error: {}", e);
                        break;
                    }
                    None => {
                        debug!("WebSocket stream ended");
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    info!("Orderbook WebSocket connection closed");
}

async fn wait_for_subscription(
    receiver: &mut futures_util::stream::SplitStream<WebSocket>,
) -> Result<WsSubscribeRequest, Box<dyn std::error::Error + Send + Sync>> {
    // Wait up to 30 seconds for subscription request
    let timeout = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    let req: WsSubscribeRequest = serde_json::from_str(&text)?;

                    if req.action != "subscribe" {
                        return Err("First message must be subscribe action".into());
                    }

                    // Validate
                    if req.symbols.is_empty() {
                        return Err("symbols cannot be empty".into());
                    }
                    if req.symbols.len() != req.weights.len() {
                        return Err("symbols and weights must have same length".into());
                    }
                    let total: u32 = req.weights.iter().sum();
                    if total != 10000 {
                        return Err(format!("weights must sum to 10000, got {}", total).into());
                    }

                    return Ok(req);
                }
                Ok(Message::Ping(_)) => {
                    // Ignore pings during setup
                    continue;
                }
                Ok(Message::Close(_)) => {
                    return Err("Connection closed before subscription".into());
                }
                Err(e) => {
                    return Err(format!("WebSocket error: {}", e).into());
                }
                _ => continue,
            }
        }
        Err("Connection ended before subscription".into())
    });

    timeout.await.map_err(|_| "Subscription timeout")?
}
