//! Live Orderbook Cache
//!
//! Maintains real-time orderbook data from Bitget WebSocket feeds.
//! Used by the WebSocket handler to stream live updates to frontend clients.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn, trace};

/// Single orderbook for an asset
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetOrderbook {
    pub symbol: String,
    pub bids: Vec<(f64, f64)>, // (price, quantity)
    pub asks: Vec<(f64, f64)>,
    pub mid_price: f64,
    pub spread_bps: f64,
    pub last_update: u64, // timestamp ms
}

impl AssetOrderbook {
    pub fn new(symbol: String) -> Self {
        Self {
            symbol,
            bids: Vec::new(),
            asks: Vec::new(),
            mid_price: 0.0,
            spread_bps: 0.0,
            last_update: 0,
        }
    }

    pub fn update(&mut self, bids: Vec<(f64, f64)>, asks: Vec<(f64, f64)>) {
        self.bids = bids;
        self.asks = asks;

        // Calculate mid price and spread
        let best_bid = self.bids.first().map(|(p, _)| *p).unwrap_or(0.0);
        let best_ask = self.asks.first().map(|(p, _)| *p).unwrap_or(0.0);

        if best_bid > 0.0 && best_ask > 0.0 {
            self.mid_price = (best_bid + best_ask) / 2.0;
            self.spread_bps = ((best_ask - best_bid) / self.mid_price) * 10000.0;
        }

        self.last_update = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
    }

    pub fn bid_depth_usd(&self, levels: usize) -> f64 {
        self.bids.iter().take(levels).map(|(p, q)| p * q).sum()
    }

    pub fn ask_depth_usd(&self, levels: usize) -> f64 {
        self.asks.iter().take(levels).map(|(p, q)| p * q).sum()
    }
}

/// Aggregated orderbook for an index composition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatedLiveOrderbook {
    pub bids: Vec<OrderbookLevel>,
    pub asks: Vec<OrderbookLevel>,
    pub mid_price: f64,
    pub spread_bps: f64,
    pub total_bid_depth_usd: f64,
    pub total_ask_depth_usd: f64,
    pub assets_included: usize,
    pub assets_failed: Vec<String>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderbookLevel {
    pub price: f64,
    pub quantity: f64,
    pub usd_value: f64,
}

/// Live orderbook cache with broadcast capability
pub struct LiveOrderbookCache {
    /// Orderbooks by symbol (e.g., "BTCUSDT" -> orderbook)
    orderbooks: Arc<RwLock<HashMap<String, AssetOrderbook>>>,
    /// Broadcast channel for updates
    update_tx: broadcast::Sender<String>, // symbol that was updated
    /// Last update time for rate limiting
    last_broadcast: Arc<RwLock<Instant>>,
}

impl LiveOrderbookCache {
    pub fn new() -> Self {
        let (update_tx, _) = broadcast::channel(1000);
        Self {
            orderbooks: Arc::new(RwLock::new(HashMap::new())),
            update_tx,
            last_broadcast: Arc::new(RwLock::new(Instant::now())),
        }
    }

    /// Subscribe to orderbook updates
    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.update_tx.subscribe()
    }

    /// Update orderbook for a symbol
    pub fn update_orderbook(&self, symbol: &str, bids: Vec<(f64, f64)>, asks: Vec<(f64, f64)>) {
        let mut orderbooks = self.orderbooks.write();
        let is_new = !orderbooks.contains_key(symbol);
        let orderbook = orderbooks
            .entry(symbol.to_string())
            .or_insert_with(|| AssetOrderbook::new(symbol.to_string()));
        orderbook.update(bids, asks);

        if is_new {
            debug!("New symbol in cache: {} (total: {})", symbol, orderbooks.len());
        }
        drop(orderbooks);

        // Broadcast update (ignore errors if no subscribers)
        let _ = self.update_tx.send(symbol.to_string());
    }

    /// Get orderbook for a symbol
    pub fn get_orderbook(&self, symbol: &str) -> Option<AssetOrderbook> {
        self.orderbooks.read().get(symbol).cloned()
    }

    /// Get aggregated orderbook for multiple symbols with weights
    /// aggregation_bps: how much price difference (in basis points) to aggregate into one level
    ///   - 1 = 0.01% (most granular)
    ///   - 10 = 0.1% (default)
    ///   - 50 = 0.5%
    ///   - 100 = 1% (most aggregated)
    /// base_mid_price: optional override for the mid price (e.g., ITP current price)
    pub fn get_aggregated(
        &self,
        symbols: &[String],
        weights: &[u32],
        levels: usize,
        aggregation_bps: u32,
        base_mid_price: Option<f64>,
    ) -> AggregatedLiveOrderbook {
        let orderbooks = self.orderbooks.read();

        let mut valid_orderbooks: Vec<(&AssetOrderbook, f64)> = Vec::new();
        let mut assets_failed: Vec<String> = Vec::new();
        let mut weighted_mid_price = 0.0;

        for (i, symbol) in symbols.iter().enumerate() {
            let weight = weights.get(i).copied().unwrap_or(0);
            let weight_pct = weight as f64 / 10000.0;

            // Try to find orderbook: USDC priority, then USDT fallback
            // Symbol may already have suffix (e.g., "BTCUSDC") or be base only (e.g., "BTC")
            let upper_symbol = symbol.to_uppercase();
            let keys_to_try = if upper_symbol.ends_with("USDC") || upper_symbol.ends_with("USDT") {
                // Already has suffix, use as-is
                vec![upper_symbol]
            } else {
                // Try USDC first (vendor preference), then USDT fallback
                vec![
                    format!("{}USDC", upper_symbol),
                    format!("{}USDT", upper_symbol),
                ]
            };

            let mut found = false;
            for key in &keys_to_try {
                if let Some(ob) = orderbooks.get(key) {
                    if ob.mid_price > 0.0 && ob.last_update > 0 {
                        weighted_mid_price += ob.mid_price * weight_pct;
                        valid_orderbooks.push((ob, weight_pct));
                        trace!("Found {} -> mid_price={}", key, ob.mid_price);
                        found = true;
                        break;
                    }
                }
            }

            if !found {
                assets_failed.push(symbol.clone());
                debug!("Symbol {} not found in cache (tried {:?}, have {} symbols)", symbol, keys_to_try, orderbooks.len());
            }
        }

        // Use base_mid_price if provided (e.g., ITP's current price)
        let final_mid_price = base_mid_price.unwrap_or(weighted_mid_price);

        if valid_orderbooks.is_empty() || final_mid_price <= 0.0 {
            return AggregatedLiveOrderbook {
                bids: Vec::new(),
                asks: Vec::new(),
                mid_price: base_mid_price.unwrap_or(0.0),
                spread_bps: 0.0,
                total_bid_depth_usd: 0.0,
                total_ask_depth_usd: 0.0,
                assets_included: 0,
                assets_failed: symbols.to_vec(),
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            };
        }

        // Aggregate orderbooks
        // Take many more levels from each orderbook to build deeper aggregate
        // (final truncation happens in aggregate_levels)
        let input_levels = levels.max(50); // Take at least 50 levels from each orderbook

        let mut all_bids: Vec<OrderbookLevel> = Vec::new();
        let mut all_asks: Vec<OrderbookLevel> = Vec::new();

        for (ob, weight) in &valid_orderbooks {
            // Skip if weight is zero to avoid division by zero
            if *weight <= 0.0 {
                continue;
            }

            // Convert to index-relative prices using final_mid_price
            for (price, qty) in ob.bids.iter().take(input_levels) {
                let relative = *price / ob.mid_price;
                let index_price = final_mid_price * relative;
                // USD value = asset_liquidity / weight
                // Because: to buy $X of index, you need $X * weight of this asset
                // So: $1M of asset with 10% weight = $10M of index capacity
                let asset_usd = price * qty;
                let usd_value = asset_usd / weight;

                all_bids.push(OrderbookLevel {
                    price: index_price,
                    quantity: usd_value / index_price,
                    usd_value,
                });
            }

            for (price, qty) in ob.asks.iter().take(input_levels) {
                let relative = *price / ob.mid_price;
                let index_price = final_mid_price * relative;
                // Same logic: asset_liquidity / weight = index capacity
                let asset_usd = price * qty;
                let usd_value = asset_usd / weight;

                all_asks.push(OrderbookLevel {
                    price: index_price,
                    quantity: usd_value / index_price,
                    usd_value,
                });
            }
        }

        // Sort and aggregate close levels
        all_bids.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));
        all_asks.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal));

        // Aggregate levels within the specified threshold
        let aggregated_bids = aggregate_levels(all_bids, levels, aggregation_bps);
        let aggregated_asks = aggregate_levels(all_asks, levels, aggregation_bps);

        let total_bid_depth: f64 = aggregated_bids.iter().map(|l| l.usd_value).sum();
        let total_ask_depth: f64 = aggregated_asks.iter().map(|l| l.usd_value).sum();

        let best_bid = aggregated_bids.first().map(|l| l.price).unwrap_or(0.0);
        let best_ask = aggregated_asks.first().map(|l| l.price).unwrap_or(0.0);
        let spread_bps = if final_mid_price > 0.0 {
            ((best_ask - best_bid) / final_mid_price) * 10000.0
        } else {
            0.0
        };

        AggregatedLiveOrderbook {
            bids: aggregated_bids,
            asks: aggregated_asks,
            mid_price: final_mid_price,
            spread_bps,
            total_bid_depth_usd: total_bid_depth,
            total_ask_depth_usd: total_ask_depth,
            assets_included: valid_orderbooks.len(),
            assets_failed,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        }
    }

    /// Get number of symbols in cache
    pub fn symbol_count(&self) -> usize {
        self.orderbooks.read().len()
    }

    /// Get all cached symbols
    pub fn symbols(&self) -> Vec<String> {
        self.orderbooks.read().keys().cloned().collect()
    }
}

impl Default for LiveOrderbookCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Aggregate orderbook levels within a price threshold
/// aggregation_bps: aggregation threshold in basis points (e.g., 10 = 0.1%)
fn aggregate_levels(levels: Vec<OrderbookLevel>, max_levels: usize, aggregation_bps: u32) -> Vec<OrderbookLevel> {
    if levels.is_empty() {
        return vec![];
    }

    let mut aggregated: Vec<OrderbookLevel> = Vec::new();
    // Convert basis points to decimal (10 bps = 0.001 = 0.1%)
    let threshold = aggregation_bps as f64 / 10000.0;

    for level in levels {
        if let Some(last) = aggregated.last_mut() {
            let price_diff = (level.price - last.price).abs() / last.price;
            if price_diff < threshold {
                last.quantity += level.quantity;
                last.usd_value += level.usd_value;
                continue;
            }
        }
        aggregated.push(level);
    }

    aggregated.truncate(max_levels);
    aggregated
}
