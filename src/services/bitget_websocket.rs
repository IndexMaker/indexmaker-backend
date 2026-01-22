//! Bitget WebSocket Multi-Subscriber
//!
//! Real-time price streaming using multiple WebSocket connections.
//! Distributes 600+ symbols across ~13 connections (50 symbols each) for stability.
//!
//! Bitget limits: 1000 channels/connection, but recommends < 50 for stability.

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio::time::interval;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use tracing::{debug, error, info, trace, warn};

/// Price data from WebSocket
#[derive(Debug, Clone)]
pub struct WsPrice {
    pub symbol: String,      // e.g., "BTCUSDT"
    pub price: f64,          // Last price
    pub bid: f64,            // Best bid
    pub ask: f64,            // Best ask
    pub timestamp_ms: u64,   // Server timestamp
}

/// Configuration for multi-websocket subscriber
#[derive(Clone)]
pub struct MultiWsConfig {
    /// WebSocket URL
    pub ws_url: String,
    /// Symbols per connection (recommended: 50)
    pub symbols_per_connection: usize,
    /// Heartbeat interval (seconds)
    pub heartbeat_interval_secs: u64,
}

impl Default for MultiWsConfig {
    fn default() -> Self {
        Self {
            ws_url: "wss://ws.bitget.com/v2/ws/public".to_string(),
            symbols_per_connection: 50,
            heartbeat_interval_secs: 25,
        }
    }
}

/// Multi-WebSocket subscriber for Bitget
#[derive(Clone)]
pub struct BitgetMultiWsSubscriber {
    config: MultiWsConfig,
    /// Symbol -> Price cache (thread-safe)
    prices: Arc<RwLock<HashMap<String, WsPrice>>>,
    /// Running flag
    running: Arc<RwLock<bool>>,
}

// WebSocket message types
#[derive(Debug, Serialize)]
struct SubscribeMessage {
    op: String,
    args: Vec<SubscribeArg>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubscribeArg {
    inst_type: String,
    channel: String,
    inst_id: String,
}

#[derive(Debug, Deserialize)]
struct WsMessage {
    action: Option<String>,
    arg: Option<ChannelArg>,
    data: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChannelArg {
    channel: String,
    inst_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TickerData {
    last_pr: String,
    bid_pr: String,
    ask_pr: String,
    ts: String,
}

impl BitgetMultiWsSubscriber {
    /// Create a new multi-websocket subscriber
    pub fn new(config: MultiWsConfig) -> Self {
        Self {
            config,
            prices: Arc::new(RwLock::new(HashMap::new())),
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// Start the subscriber with given symbols
    pub async fn start(&self, symbols: Vec<String>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if symbols.is_empty() {
            warn!("No symbols provided, not starting");
            return Ok(());
        }

        // Mark as running
        *self.running.write().await = true;

        let chunk_size = self.config.symbols_per_connection;
        let chunks: Vec<Vec<String>> = symbols
            .chunks(chunk_size)
            .map(|c| c.to_vec())
            .collect();

        let num_connections = chunks.len();
        info!(
            "Starting Bitget multi-websocket: {} symbols across {} connections ({} symbols/connection)",
            symbols.len(),
            num_connections,
            chunk_size
        );

        // Spawn connection tasks
        for (idx, chunk) in chunks.into_iter().enumerate() {
            let config = self.config.clone();
            let prices = self.prices.clone();
            let running = self.running.clone();
            let connection_id = idx + 1;

            tokio::spawn(async move {
                loop {
                    if !*running.read().await {
                        info!("Connection {} stopping (shutdown)", connection_id);
                        break;
                    }

                    match run_connection(
                        connection_id,
                        num_connections,
                        &config,
                        &chunk,
                        prices.clone(),
                        running.clone(),
                    )
                    .await
                    {
                        Ok(()) => {
                            if !*running.read().await {
                                break;
                            }
                            warn!("Connection {} ended, reconnecting...", connection_id);
                        }
                        Err(e) => {
                            if !*running.read().await {
                                break;
                            }
                            error!("Connection {} failed: {}, reconnecting in 5s...", connection_id, e);
                            tokio::time::sleep(Duration::from_secs(5)).await;
                        }
                    }
                }
            });
        }

        Ok(())
    }

    /// Stop all connections
    pub async fn stop(&self) {
        info!("Stopping Bitget multi-websocket subscriber");
        *self.running.write().await = false;
    }

    /// Get price for a symbol
    pub async fn get_price(&self, symbol: &str) -> Option<f64> {
        let prices = self.prices.read().await;
        prices.get(symbol).map(|p| p.price)
    }

    /// Get all prices
    pub async fn get_all_prices(&self) -> HashMap<String, f64> {
        let prices = self.prices.read().await;
        prices.iter().map(|(k, v)| (k.clone(), v.price)).collect()
    }

    /// Get full price data for a symbol
    pub async fn get_price_data(&self, symbol: &str) -> Option<WsPrice> {
        let prices = self.prices.read().await;
        prices.get(symbol).cloned()
    }

    /// Get price count
    pub async fn get_price_count(&self) -> usize {
        self.prices.read().await.len()
    }

    /// Check if running
    pub async fn is_running(&self) -> bool {
        *self.running.read().await
    }
}

/// Run a single WebSocket connection
async fn run_connection(
    connection_id: usize,
    total_connections: usize,
    config: &MultiWsConfig,
    symbols: &[String],
    prices: Arc<RwLock<HashMap<String, WsPrice>>>,
    running: Arc<RwLock<bool>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!(
        "Connection {}/{}: Connecting to {} for {} symbols",
        connection_id, total_connections, config.ws_url, symbols.len()
    );

    // Connect
    let (ws_stream, _) = connect_async(&config.ws_url).await?;
    let (mut write, mut read) = ws_stream.split();

    info!(
        "Connection {}/{}: Connected, subscribing to {} symbols",
        connection_id, total_connections, symbols.len()
    );

    // Batch subscribe
    let subscribe_msg = create_batch_subscribe(symbols);
    let json = serde_json::to_string(&subscribe_msg)?;
    write.send(Message::Text(json)).await?;

    info!(
        "Connection {}/{}: Subscribed to {} symbols (first={}, last={})",
        connection_id,
        total_connections,
        symbols.len(),
        symbols.first().unwrap_or(&"".to_string()),
        symbols.last().unwrap_or(&"".to_string())
    );

    // Heartbeat timer
    let mut heartbeat_timer = interval(Duration::from_secs(config.heartbeat_interval_secs));
    let mut last_pong = std::time::Instant::now();

    loop {
        if !*running.read().await {
            break;
        }

        tokio::select! {
            _ = heartbeat_timer.tick() => {
                // Send ping
                write.send(Message::Text("ping".to_string())).await?;
                trace!("Connection {}: Sent ping", connection_id);

                // Check pong timeout (2 minutes)
                if last_pong.elapsed() > Duration::from_secs(120) {
                    error!("Connection {}: No pong for 2 minutes", connection_id);
                    return Err("Pong timeout".into());
                }
            }

            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        // Handle pong
                        if text.trim() == "pong" {
                            last_pong = std::time::Instant::now();
                            trace!("Connection {}: Received pong", connection_id);
                            continue;
                        }

                        // Parse message
                        if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                            if let Err(e) = handle_message(&ws_msg, &prices).await {
                                trace!("Connection {}: Message handling: {}", connection_id, e);
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!("Connection {}: WebSocket closed", connection_id);
                        break;
                    }
                    Some(Err(e)) => {
                        error!("Connection {}: WebSocket error: {}", connection_id, e);
                        return Err(e.into());
                    }
                    None => {
                        info!("Connection {}: Stream ended", connection_id);
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

/// Create batch subscribe message
fn create_batch_subscribe(symbols: &[String]) -> SubscribeMessage {
    let args: Vec<SubscribeArg> = symbols
        .iter()
        .map(|s| SubscribeArg {
            inst_type: "SPOT".to_string(),
            channel: "ticker".to_string(),
            inst_id: s.to_uppercase(),
        })
        .collect();

    SubscribeMessage {
        op: "subscribe".to_string(),
        args,
    }
}

/// Handle incoming WebSocket message
async fn handle_message(
    msg: &WsMessage,
    prices: &Arc<RwLock<HashMap<String, WsPrice>>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let arg = msg.arg.as_ref().ok_or("Missing arg")?;
    let data = msg.data.as_ref().ok_or("Missing data")?;

    if data.is_empty() || arg.channel != "ticker" {
        return Ok(());
    }

    // Parse ticker data
    let ticker: TickerData = serde_json::from_value(data[0].clone())?;

    let price = ticker.last_pr.parse::<f64>().unwrap_or(0.0);
    let bid = ticker.bid_pr.parse::<f64>().unwrap_or(0.0);
    let ask = ticker.ask_pr.parse::<f64>().unwrap_or(0.0);
    let ts = ticker.ts.parse::<u64>().unwrap_or(0);

    let ws_price = WsPrice {
        symbol: arg.inst_id.clone(),
        price,
        bid,
        ask,
        timestamp_ms: ts,
    };

    // Update cache
    prices.write().await.insert(arg.inst_id.clone(), ws_price);

    Ok(())
}

/// Fetch all available USDT symbols from Bitget
pub async fn fetch_all_usdt_symbols() -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::new();
    let response = client
        .get("https://api.bitget.com/api/v2/spot/public/symbols")
        .send()
        .await?;

    #[derive(Deserialize)]
    struct SymbolsResponse {
        data: Vec<SymbolData>,
    }

    #[derive(Deserialize)]
    struct SymbolData {
        symbol: String,
        status: String,
    }

    let data: SymbolsResponse = response.json().await?;

    let symbols: Vec<String> = data
        .data
        .into_iter()
        .filter(|s| s.symbol.ends_with("USDT") && s.status == "online")
        .map(|s| s.symbol)
        .collect();

    info!("Fetched {} USDT symbols from Bitget", symbols.len());
    Ok(symbols)
}

impl Default for BitgetMultiWsSubscriber {
    fn default() -> Self {
        Self::new(MultiWsConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = MultiWsConfig::default();
        assert_eq!(config.symbols_per_connection, 50);
        assert_eq!(config.heartbeat_interval_secs, 25);
    }

    #[test]
    fn test_batch_subscribe() {
        let symbols = vec!["BTCUSDT".to_string(), "ETHUSDT".to_string()];
        let msg = create_batch_subscribe(&symbols);
        assert_eq!(msg.op, "subscribe");
        assert_eq!(msg.args.len(), 2);
        assert_eq!(msg.args[0].channel, "ticker");
    }

    #[tokio::test]
    async fn test_fetch_symbols() {
        let symbols = fetch_all_usdt_symbols().await;
        assert!(symbols.is_ok());
        let syms = symbols.unwrap();
        assert!(syms.len() > 500); // Should be 600+
        assert!(syms.contains(&"BTCUSDT".to_string()));
    }
}
