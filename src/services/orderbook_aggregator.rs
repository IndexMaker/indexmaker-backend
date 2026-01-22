//! Orderbook Aggregator Service
//!
//! Fetches orderbooks from Bitget and aggregates them into a virtual orderbook
//! for an index composition. Used to preview index liquidity before deployment.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// Orderbook level (price, quantity, source)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderbookLevel {
    pub price: f64,
    pub quantity: f64,
    /// USD value at this level (price * quantity)
    pub usd_value: f64,
    /// Source asset(s) contributing to this level
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<String>>,
}

/// Single asset orderbook
#[derive(Debug, Clone)]
pub struct AssetOrderbook {
    pub symbol: String,
    pub bids: Vec<(f64, f64)>, // (price, quantity)
    pub asks: Vec<(f64, f64)>,
    pub mid_price: f64,
}

/// Aggregated virtual orderbook for an index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatedOrderbook {
    /// Bid levels (highest price first)
    pub bids: Vec<OrderbookLevel>,
    /// Ask levels (lowest price first)
    pub asks: Vec<OrderbookLevel>,
    /// Weighted mid price of the index
    pub mid_price: f64,
    /// Spread in basis points
    pub spread_bps: f64,
    /// Total bid depth in USD
    pub total_bid_depth_usd: f64,
    /// Total ask depth in USD
    pub total_ask_depth_usd: f64,
    /// Number of assets included
    pub assets_included: usize,
    /// Assets that failed to fetch
    pub assets_failed: Vec<String>,
    /// Per-asset breakdown
    pub asset_details: Vec<AssetOrderbookSummary>,
}

/// Summary of a single asset's orderbook
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetOrderbookSummary {
    pub symbol: String,
    pub weight_bps: u32,
    pub mid_price: f64,
    pub spread_bps: f64,
    pub bid_depth_usd: f64,
    pub ask_depth_usd: f64,
}

/// Request for aggregated orderbook
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateOrderbookRequest {
    /// Asset symbols (e.g., ["BTC", "ETH", "SOL"])
    pub symbols: Vec<String>,
    /// Weights in basis points (must sum to 10000)
    pub weights: Vec<u32>,
    /// Number of levels to return (default: 10)
    #[serde(default = "default_levels")]
    pub levels: usize,
    /// Quote asset (default: "USDT")
    #[serde(default = "default_quote")]
    pub quote: String,
}

fn default_levels() -> usize {
    10
}

fn default_quote() -> String {
    "USDT".to_string()
}

/// Bitget orderbook API response
#[derive(Debug, Deserialize)]
struct BitgetOrderbookResponse {
    code: String,
    data: Option<BitgetOrderbookData>,
}

#[derive(Debug, Deserialize)]
struct BitgetOrderbookData {
    bids: Vec<Vec<String>>,
    asks: Vec<Vec<String>>,
    ts: String,
}

/// Orderbook aggregator service
#[derive(Clone)]
pub struct OrderbookAggregator {
    client: Client,
}

impl OrderbookAggregator {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
        }
    }

    /// Fetch orderbook for a single symbol from Bitget
    pub async fn fetch_orderbook(
        &self,
        symbol: &str,
        quote: &str,
        levels: usize,
    ) -> Result<AssetOrderbook, Box<dyn std::error::Error + Send + Sync>> {
        let trading_pair = format!("{}{}", symbol.to_uppercase(), quote.to_uppercase());
        let url = format!(
            "https://api.bitget.com/api/v2/spot/market/orderbook?symbol={}&limit={}",
            trading_pair,
            levels.min(150) // Bitget max is 150
        );

        debug!("Fetching orderbook for {}", trading_pair);

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(format!("HTTP error {} for {}", response.status(), trading_pair).into());
        }

        let data: BitgetOrderbookResponse = response.json().await?;

        if data.code != "00000" {
            return Err(format!("Bitget API error for {}: code {}", trading_pair, data.code).into());
        }

        let orderbook_data = data.data.ok_or("No orderbook data")?;

        // Parse bids and asks
        let bids: Vec<(f64, f64)> = orderbook_data
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

        let asks: Vec<(f64, f64)> = orderbook_data
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

        // Calculate mid price
        let best_bid = bids.first().map(|(p, _)| *p).unwrap_or(0.0);
        let best_ask = asks.first().map(|(p, _)| *p).unwrap_or(0.0);
        let mid_price = if best_bid > 0.0 && best_ask > 0.0 {
            (best_bid + best_ask) / 2.0
        } else {
            0.0
        };

        Ok(AssetOrderbook {
            symbol: symbol.to_uppercase(),
            bids,
            asks,
            mid_price,
        })
    }

    /// Fetch orderbooks for multiple symbols concurrently
    pub async fn fetch_orderbooks(
        &self,
        symbols: &[String],
        quote: &str,
        levels: usize,
    ) -> HashMap<String, Result<AssetOrderbook, String>> {
        let futures: Vec<_> = symbols
            .iter()
            .map(|symbol| {
                let symbol = symbol.clone();
                let quote = quote.to_string();
                async move {
                    let result = self.fetch_orderbook(&symbol, &quote, levels).await;
                    (symbol, result.map_err(|e| e.to_string()))
                }
            })
            .collect();

        let results = futures_util::future::join_all(futures).await;
        results.into_iter().collect()
    }

    /// Aggregate orderbooks into a virtual index orderbook
    pub async fn aggregate_orderbooks(
        &self,
        request: &AggregateOrderbookRequest,
    ) -> Result<AggregatedOrderbook, Box<dyn std::error::Error + Send + Sync>> {
        // Validate request
        if request.symbols.len() != request.weights.len() {
            return Err("symbols and weights must have same length".into());
        }

        let total_weight: u32 = request.weights.iter().sum();
        if total_weight != 10000 {
            return Err(format!("weights must sum to 10000, got {}", total_weight).into());
        }

        info!(
            "Aggregating orderbooks for {} assets",
            request.symbols.len()
        );

        // Fetch all orderbooks concurrently
        let orderbooks = self
            .fetch_orderbooks(&request.symbols, &request.quote, request.levels * 2)
            .await;

        // Process results
        let mut asset_details = Vec::new();
        let mut assets_failed = Vec::new();
        let mut weighted_mid_price = 0.0;

        // Collect successful orderbooks with their weights
        let mut valid_orderbooks: Vec<(AssetOrderbook, f64)> = Vec::new();

        for (i, symbol) in request.symbols.iter().enumerate() {
            let weight = request.weights[i];
            let weight_pct = weight as f64 / 10000.0;

            match orderbooks.get(symbol) {
                Some(Ok(ob)) => {
                    // Calculate spread
                    let best_bid = ob.bids.first().map(|(p, _)| *p).unwrap_or(0.0);
                    let best_ask = ob.asks.first().map(|(p, _)| *p).unwrap_or(0.0);
                    let spread_bps = if ob.mid_price > 0.0 {
                        ((best_ask - best_bid) / ob.mid_price) * 10000.0
                    } else {
                        0.0
                    };

                    // Calculate depth
                    let bid_depth: f64 = ob.bids.iter().map(|(p, q)| p * q).sum();
                    let ask_depth: f64 = ob.asks.iter().map(|(p, q)| p * q).sum();

                    asset_details.push(AssetOrderbookSummary {
                        symbol: symbol.clone(),
                        weight_bps: weight,
                        mid_price: ob.mid_price,
                        spread_bps,
                        bid_depth_usd: bid_depth,
                        ask_depth_usd: ask_depth,
                    });

                    weighted_mid_price += ob.mid_price * weight_pct;
                    valid_orderbooks.push((ob.clone(), weight_pct));
                }
                Some(Err(e)) => {
                    warn!("Failed to fetch orderbook for {}: {}", symbol, e);
                    assets_failed.push(symbol.clone());
                }
                None => {
                    assets_failed.push(symbol.clone());
                }
            }
        }

        if valid_orderbooks.is_empty() {
            return Err("No valid orderbooks fetched".into());
        }

        // Aggregate orderbooks
        // For bids: combine all bids, weight by composition, sort by price desc
        // For asks: combine all asks, weight by composition, sort by price asc
        let (aggregated_bids, aggregated_asks) =
            self.combine_orderbooks(&valid_orderbooks, weighted_mid_price, request.levels);

        // Calculate totals
        let total_bid_depth: f64 = aggregated_bids.iter().map(|l| l.usd_value).sum();
        let total_ask_depth: f64 = aggregated_asks.iter().map(|l| l.usd_value).sum();

        // Calculate spread
        let best_bid = aggregated_bids.first().map(|l| l.price).unwrap_or(0.0);
        let best_ask = aggregated_asks.first().map(|l| l.price).unwrap_or(0.0);
        let spread_bps = if weighted_mid_price > 0.0 {
            ((best_ask - best_bid) / weighted_mid_price) * 10000.0
        } else {
            0.0
        };

        Ok(AggregatedOrderbook {
            bids: aggregated_bids,
            asks: aggregated_asks,
            mid_price: weighted_mid_price,
            spread_bps,
            total_bid_depth_usd: total_bid_depth,
            total_ask_depth_usd: total_ask_depth,
            assets_included: valid_orderbooks.len(),
            assets_failed,
            asset_details,
        })
    }

    /// Combine multiple orderbooks into aggregated bid/ask levels
    fn combine_orderbooks(
        &self,
        orderbooks: &[(AssetOrderbook, f64)], // (orderbook, weight_pct)
        index_mid_price: f64,
        levels: usize,
    ) -> (Vec<OrderbookLevel>, Vec<OrderbookLevel>) {
        // Strategy: Convert each asset's orderbook to "index-relative" prices
        // For each level, calculate what % away from mid it is, then apply to index mid

        let mut all_bids: Vec<OrderbookLevel> = Vec::new();
        let mut all_asks: Vec<OrderbookLevel> = Vec::new();

        for (ob, weight) in orderbooks {
            if ob.mid_price <= 0.0 {
                continue;
            }

            // Process bids: convert to relative levels
            for (price, qty) in &ob.bids {
                // Calculate relative price (as % of mid)
                let relative = *price / ob.mid_price;
                // Apply to index mid
                let index_price = index_mid_price * relative;
                // Weight the quantity (in USD terms)
                let usd_value = price * qty * weight;
                let weighted_qty = usd_value / index_price;

                all_bids.push(OrderbookLevel {
                    price: index_price,
                    quantity: weighted_qty,
                    usd_value,
                    sources: Some(vec![ob.symbol.clone()]),
                });
            }

            // Process asks
            for (price, qty) in &ob.asks {
                let relative = *price / ob.mid_price;
                let index_price = index_mid_price * relative;
                let usd_value = price * qty * weight;
                let weighted_qty = usd_value / index_price;

                all_asks.push(OrderbookLevel {
                    price: index_price,
                    quantity: weighted_qty,
                    usd_value,
                    sources: Some(vec![ob.symbol.clone()]),
                });
            }
        }

        // Aggregate levels that are close together (within 0.1%)
        let aggregated_bids = self.aggregate_levels(all_bids, true, levels);
        let aggregated_asks = self.aggregate_levels(all_asks, false, levels);

        (aggregated_bids, aggregated_asks)
    }

    /// Aggregate price levels that are close together
    fn aggregate_levels(
        &self,
        mut levels: Vec<OrderbookLevel>,
        is_bid: bool,
        max_levels: usize,
    ) -> Vec<OrderbookLevel> {
        if levels.is_empty() {
            return vec![];
        }

        // Sort: bids descending, asks ascending
        if is_bid {
            levels.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            levels.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal));
        }

        // Group levels within 0.01% of each other (very tight to preserve more levels)
        let mut aggregated: Vec<OrderbookLevel> = Vec::new();
        let threshold = 0.0001; // 0.01%

        for level in levels {
            if let Some(last) = aggregated.last_mut() {
                let price_diff = (level.price - last.price).abs() / last.price;
                if price_diff < threshold {
                    // Merge into existing level
                    let total_usd = last.usd_value + level.usd_value;
                    last.quantity += level.quantity;
                    last.usd_value = total_usd;
                    // Average the price weighted by USD value
                    last.price = (last.price * (last.usd_value - level.usd_value) + level.price * level.usd_value) / total_usd;
                    // Merge sources
                    if let (Some(sources), Some(new_sources)) = (&mut last.sources, level.sources) {
                        for s in new_sources {
                            if !sources.contains(&s) {
                                sources.push(s);
                            }
                        }
                    }
                    continue;
                }
            }
            aggregated.push(level);
        }

        // Limit to max_levels
        aggregated.truncate(max_levels);

        // Clean up sources for display
        for level in &mut aggregated {
            level.sources = None; // Remove sources for cleaner output
        }

        aggregated
    }
}

impl Default for OrderbookAggregator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fetch_single_orderbook() {
        let aggregator = OrderbookAggregator::new();
        let result = aggregator.fetch_orderbook("BTC", "USDT", 5).await;

        assert!(result.is_ok());
        let ob = result.unwrap();
        assert_eq!(ob.symbol, "BTC");
        assert!(!ob.bids.is_empty());
        assert!(!ob.asks.is_empty());
        assert!(ob.mid_price > 0.0);
        println!("BTC mid price: ${:.2}", ob.mid_price);
    }

    #[tokio::test]
    async fn test_aggregate_orderbooks() {
        let aggregator = OrderbookAggregator::new();

        let request = AggregateOrderbookRequest {
            symbols: vec!["BTC".to_string(), "ETH".to_string()],
            weights: vec![6000, 4000], // 60% BTC, 40% ETH
            levels: 5,
            quote: "USDT".to_string(),
        };

        let result = aggregator.aggregate_orderbooks(&request).await;
        assert!(result.is_ok());

        let agg = result.unwrap();
        assert_eq!(agg.assets_included, 2);
        assert!(agg.mid_price > 0.0);
        assert!(!agg.bids.is_empty());
        assert!(!agg.asks.is_empty());

        println!("Aggregated mid price: ${:.2}", agg.mid_price);
        println!("Spread: {:.2} bps", agg.spread_bps);
        println!("Bid depth: ${:.2}", agg.total_bid_depth_usd);
        println!("Ask depth: ${:.2}", agg.total_ask_depth_usd);
    }
}
