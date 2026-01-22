//! Bitget WebSocket Feeder
//!
//! Connects to Bitget WebSocket API and feeds live orderbook data into the cache.
//! Manages multiple WebSocket connections to handle 600+ symbols.

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{interval, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

use super::live_orderbook_cache::LiveOrderbookCache;

const BITGET_WS_URL: &str = "wss://ws.bitget.com/v2/ws/public";
const SYMBOLS_PER_CONNECTION: usize = 50;
const RECONNECT_DELAY: Duration = Duration::from_secs(3);
const PING_INTERVAL: Duration = Duration::from_secs(25);

/// Bitget WebSocket subscription message
#[derive(Debug, Serialize)]
struct SubscribeMessage {
    op: String,
    args: Vec<SubscribeArg>,
}

#[derive(Debug, Serialize)]
struct SubscribeArg {
    #[serde(rename = "instType")]
    inst_type: String,
    channel: String,
    #[serde(rename = "instId")]
    inst_id: String,
}

/// Bitget WebSocket message types
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WsMessage {
    Event(EventMessage),
    Data(DataMessage),
    Pong(PongMessage),
}

#[derive(Debug, Deserialize)]
struct EventMessage {
    event: String,
    arg: Option<serde_json::Value>,
    code: Option<String>,
    msg: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DataMessage {
    action: Option<String>,
    arg: ArgInfo,
    data: Vec<OrderbookData>,
    ts: Option<u64>,  // Root level ts is a number
}

#[derive(Debug, Deserialize)]
struct ArgInfo {
    #[serde(rename = "instType")]
    inst_type: String,
    channel: String,
    #[serde(rename = "instId")]
    inst_id: String,
}

#[derive(Debug, Deserialize)]
struct OrderbookData {
    bids: Vec<Vec<String>>,
    asks: Vec<Vec<String>>,
    ts: Option<String>,
    checksum: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct PongMessage {
    pong: Option<String>,
}

/// Bitget WebSocket feeder that maintains connections and feeds the cache
pub struct BitgetWsFeeder {
    cache: Arc<LiveOrderbookCache>,
    symbols: Vec<String>,
}

impl BitgetWsFeeder {
    pub fn new(cache: Arc<LiveOrderbookCache>) -> Self {
        Self {
            cache,
            symbols: Vec::new(),
        }
    }

    /// Start the feeder with the given symbols
    pub async fn start(&mut self, symbols: Vec<String>) {
        self.symbols = symbols;

        if self.symbols.is_empty() {
            warn!("No symbols to subscribe to");
            return;
        }

        info!(
            "Starting Bitget WebSocket feeder for {} symbols",
            self.symbols.len()
        );

        // Split symbols into chunks for multiple connections
        let chunks: Vec<Vec<String>> = self
            .symbols
            .chunks(SYMBOLS_PER_CONNECTION)
            .map(|c| c.to_vec())
            .collect();

        info!(
            "Creating {} WebSocket connections ({} symbols per connection)",
            chunks.len(),
            SYMBOLS_PER_CONNECTION
        );

        // Spawn a task for each connection
        for (i, chunk) in chunks.into_iter().enumerate() {
            let cache = self.cache.clone();
            tokio::spawn(async move {
                run_connection(i, chunk, cache).await;
            });
        }
    }
}

/// Run a single WebSocket connection with auto-reconnect
async fn run_connection(conn_id: usize, symbols: Vec<String>, cache: Arc<LiveOrderbookCache>) {
    loop {
        info!(
            "[WS-{}] Connecting to Bitget for {} symbols...",
            conn_id,
            symbols.len()
        );

        match connect_and_subscribe(conn_id, &symbols, cache.clone()).await {
            Ok(_) => {
                info!("[WS-{}] Connection closed normally", conn_id);
            }
            Err(e) => {
                error!("[WS-{}] Connection error: {}", conn_id, e);
            }
        }

        info!("[WS-{}] Reconnecting in {:?}...", conn_id, RECONNECT_DELAY);
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

/// Connect to Bitget and subscribe to orderbook channels
async fn connect_and_subscribe(
    conn_id: usize,
    symbols: &[String],
    cache: Arc<LiveOrderbookCache>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (ws_stream, _) = connect_async(BITGET_WS_URL).await?;
    let (mut write, mut read) = ws_stream.split();

    info!("[WS-{}] Connected, subscribing to {} symbols", conn_id, symbols.len());

    // Build subscription message
    let args: Vec<SubscribeArg> = symbols
        .iter()
        .map(|symbol| SubscribeArg {
            inst_type: "SPOT".to_string(),
            channel: "books15".to_string(), // Top 15 levels for deeper orderbook
            inst_id: symbol.clone(),
        })
        .collect();

    let subscribe_msg = SubscribeMessage {
        op: "subscribe".to_string(),
        args,
    };

    let json = serde_json::to_string(&subscribe_msg)?;
    write.send(Message::Text(json)).await?;

    // Start ping task
    let (ping_tx, mut ping_rx) = mpsc::channel::<()>(1);
    tokio::spawn(async move {
        let mut ticker = interval(PING_INTERVAL);
        loop {
            ticker.tick().await;
            if ping_tx.send(()).await.is_err() {
                break;
            }
        }
    });

    // Main message loop
    loop {
        tokio::select! {
            // Handle incoming messages
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Err(e) = process_message(conn_id, &text, &cache) {
                            debug!("[WS-{}] Error processing message: {}", conn_id, e);
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        write.send(Message::Pong(data)).await?;
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!("[WS-{}] Received close frame", conn_id);
                        break;
                    }
                    Some(Err(e)) => {
                        error!("[WS-{}] WebSocket error: {}", conn_id, e);
                        break;
                    }
                    None => {
                        info!("[WS-{}] Stream ended", conn_id);
                        break;
                    }
                    _ => {}
                }
            }

            // Send ping
            _ = ping_rx.recv() => {
                write.send(Message::Text("ping".to_string())).await?;
            }
        }
    }

    Ok(())
}

/// Process incoming WebSocket message
fn process_message(
    conn_id: usize,
    text: &str,
    cache: &LiveOrderbookCache,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Handle pong
    if text == "pong" {
        return Ok(());
    }

    // Log first data message for debugging
    static LOGGED_DATA: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if text.contains("\"data\"") && !LOGGED_DATA.swap(true, std::sync::atomic::Ordering::Relaxed) {
        info!("[WS] Sample DATA message (first 800 chars): {}", &text[..text.len().min(800)]);
    }

    let msg: WsMessage = serde_json::from_str(text)?;

    match msg {
        WsMessage::Event(event) => {
            if event.event == "subscribe" {
                debug!("[WS-{}] Subscription confirmed", conn_id);
            } else if event.event == "error" {
                warn!(
                    "[WS-{}] Error: {} - {}",
                    conn_id,
                    event.code.unwrap_or_default(),
                    event.msg.unwrap_or_default()
                );
            }
        }
        WsMessage::Data(data) => {
            let symbol = data.arg.inst_id.clone();
            debug!("[WS-{}] Received data for {}", conn_id, symbol);

            for orderbook in data.data {
                // Parse bids
                let bids: Vec<(f64, f64)> = orderbook
                    .bids
                    .iter()
                    .filter_map(|level| {
                        if level.len() >= 2 {
                            let price = level[0].parse::<f64>().ok()?;
                            let qty = level[1].parse::<f64>().ok()?;
                            Some((price, qty))
                        } else {
                            None
                        }
                    })
                    .collect();

                // Parse asks
                let asks: Vec<(f64, f64)> = orderbook
                    .asks
                    .iter()
                    .filter_map(|level| {
                        if level.len() >= 2 {
                            let price = level[0].parse::<f64>().ok()?;
                            let qty = level[1].parse::<f64>().ok()?;
                            Some((price, qty))
                        } else {
                            None
                        }
                    })
                    .collect();

                // Update cache
                cache.update_orderbook(&symbol, bids, asks);
            }
        }
        WsMessage::Pong(_) => {
            // Pong received
        }
    }

    Ok(())
}

/// Load trading symbols from vendor's assets.json (USDC priority, single source of truth)
/// Path: ../vendor/configs/dev/assets.json relative to indexmaker-backend
pub fn load_symbols_from_vendor_assets() -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    use std::collections::HashMap;
    use std::path::Path;

    // Try multiple paths to find assets.json
    let possible_paths = [
        "../vendor/configs/dev/assets.json",
        "vendor/configs/dev/assets.json",
        "/Users/maxguillabert/Desktop/feb26/vendor/configs/dev/assets.json",
    ];

    let mut assets_path = None;
    for path in &possible_paths {
        if Path::new(path).exists() {
            assets_path = Some(path.to_string());
            break;
        }
    }

    let path = assets_path.ok_or("Could not find vendor/configs/dev/assets.json")?;
    info!("Loading trading symbols from: {}", path);

    let content = std::fs::read_to_string(&path)?;
    let assets: HashMap<String, u32> = serde_json::from_str(&content)?;

    // Keys are full symbols like "BTCUSDC", "ETHUSDC", "1INCHUSDT"
    let symbols: Vec<String> = assets.keys().cloned().collect();

    info!("Loaded {} trading symbols from vendor assets.json (USDC priority)", symbols.len());

    Ok(symbols)
}

/// Fetch all USDT trading pairs from Bitget (deprecated - use load_symbols_from_vendor_assets)
#[deprecated(note = "Use load_symbols_from_vendor_assets() for USDC priority")]
pub async fn fetch_all_usdt_symbols() -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>>
{
    let url = "https://api.bitget.com/api/v2/spot/public/symbols";

    let client = reqwest::Client::new();
    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()).into());
    }

    #[derive(Deserialize)]
    struct ApiResponse {
        code: String,
        data: Vec<SymbolInfo>,
    }

    #[derive(Deserialize)]
    struct SymbolInfo {
        symbol: String,
        #[serde(rename = "quoteCoin")]
        quote_coin: String,
        status: String,
    }

    let api_response: ApiResponse = response.json().await?;

    if api_response.code != "00000" {
        return Err(format!("API error: {}", api_response.code).into());
    }

    let symbols: Vec<String> = api_response
        .data
        .into_iter()
        .filter(|s| s.quote_coin == "USDT" && s.status == "online")
        .map(|s| s.symbol)
        .collect();

    info!("Found {} USDT trading pairs on Bitget", symbols.len());

    Ok(symbols)
}
