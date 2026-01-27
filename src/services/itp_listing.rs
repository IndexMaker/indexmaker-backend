//! ITP Listing Service
//!
//! Service for fetching ITP listings with real-time price calculation
//! from constituent asset prices provided by the real-time price service.

use chrono::{NaiveDate, Utc};
use rust_decimal::prelude::ToPrimitive;
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, EntityTrait, Order, PaginatorTrait, QueryFilter, QueryOrder,
    sea_query::{Expr, Func},
};
use std::collections::HashMap;
use tracing::{debug, info, warn};

use crate::entities::{coins_historical_prices, itps, prelude::{CoinsHistoricalPrices, Itps}};
use crate::models::itp_listing::{ItpListEntry, ItpListQuery};

/// Symbol mapping from ITP assets to exchange symbols
fn normalize_symbol(symbol: &str) -> &str {
    match symbol.to_uppercase().as_str() {
        "BTC" | "BITCOIN" => "BTC",
        "ETH" | "ETHEREUM" => "ETH",
        "SOL" | "SOLANA" => "SOL",
        "LINK" | "CHAINLINK" => "LINK",
        "UNI" | "UNISWAP" => "UNI",
        _ => symbol,
    }
}

/// ITP Listing Service - calculates prices using real-time data from exchanges
#[derive(Clone, Default)]
pub struct ItpListingService;

impl ItpListingService {
    /// Create a new ITP listing service
    pub fn new() -> Self {
        Self
    }

    /// Get all ITPs with prices calculated from real-time asset prices
    pub async fn get_all_itps(
        &self,
        db: &DatabaseConnection,
        realtime_prices: &HashMap<String, f64>,
    ) -> Result<Vec<ItpListEntry>, sea_orm::DbErr> {
        self.query_all_itps(db, realtime_prices).await
    }

    /// Get ITPs with filters and pagination
    pub async fn get_itps(
        &self,
        db: &DatabaseConnection,
        query: &ItpListQuery,
        realtime_prices: &HashMap<String, f64>,
    ) -> Result<(Vec<ItpListEntry>, i64), sea_orm::DbErr> {
        let has_filters = query.active.is_some()
            || query.search.is_some()
            || query.user_holdings.is_some()
            || query.admin_address.is_some();

        let limit = query.limit.unwrap_or(20).min(100) as u64;
        let offset = query.offset.unwrap_or(0) as u64;

        // If no filters, get all and paginate
        if !has_filters {
            let all_itps = self.get_all_itps(db, realtime_prices).await?;
            let total = all_itps.len() as i64;

            let start = offset as usize;
            let paginated: Vec<ItpListEntry> = all_itps
                .into_iter()
                .skip(start)
                .take(limit as usize)
                .collect();

            return Ok((paginated, total));
        }

        // With filters, query database directly
        let (itps, total) = self.query_itps_with_filters(db, query, limit, offset, realtime_prices).await?;
        Ok((itps, total))
    }

    /// Query all ITPs from database with real-time price calculation
    async fn query_all_itps(
        &self,
        db: &DatabaseConnection,
        realtime_prices: &HashMap<String, f64>,
    ) -> Result<Vec<ItpListEntry>, sea_orm::DbErr> {
        // Exclude deprecated ITPs (state=3) from default listing
        let itps = Itps::find()
            .filter(itps::Column::State.ne(3i16))
            .order_by_desc(itps::Column::CreatedAt)
            .all(db)
            .await?;

        info!("Using {} real-time prices for ITP calculation", realtime_prices.len());

        // Calculate prices for each ITP
        let mut entries = Vec::new();
        for itp in itps {
            let entry = calculate_itp_price(itp, realtime_prices, db).await;
            entries.push(entry);
        }

        Ok(entries)
    }

    /// Query ITPs with filters and pagination
    async fn query_itps_with_filters(
        &self,
        db: &DatabaseConnection,
        query: &ItpListQuery,
        limit: u64,
        offset: u64,
        realtime_prices: &HashMap<String, f64>,
    ) -> Result<(Vec<ItpListEntry>, i64), sea_orm::DbErr> {
        let mut select = Itps::find();

        if let Some(true) = query.active {
            select = select.filter(itps::Column::State.eq(1_i16));
        }

        if let Some(ref search) = query.search {
            let search_pattern = format!("%{}%", search);
            select = select.filter(
                Condition::any()
                    .add(Expr::expr(Func::lower(Expr::col(itps::Column::Name)))
                        .like(search_pattern.to_lowercase()))
                    .add(Expr::expr(Func::lower(Expr::col(itps::Column::Symbol)))
                        .like(search_pattern.to_lowercase()))
            );
        }

        // Story 2-3 AC#6: Filter by admin address for issuer portfolio view
        if let Some(ref admin) = query.admin_address {
            select = select.filter(itps::Column::AdminAddress.eq(admin));
        }

        let total = select.clone().count(db).await? as i64;

        use sea_orm::QuerySelect as _;
        let itps = select
            .order_by_desc(itps::Column::CreatedAt)
            .offset(offset)
            .limit(limit)
            .all(db)
            .await?;

        let mut entries = Vec::new();
        for itp in itps {
            let entry = calculate_itp_price(itp, realtime_prices, db).await;
            entries.push(entry);
        }

        Ok((entries, total))
    }

    /// Invalidate cache (no-op since we use real-time pricing)
    pub async fn invalidate_cache(&self) {
        info!("ITP listing cache invalidation called (no-op - real-time pricing)");
    }
}

/// Get base prices for assets at a specific date from historical data
async fn fetch_base_prices_at_date(
    db: &DatabaseConnection,
    symbols: &[String],
    date: NaiveDate
) -> Result<HashMap<String, f64>, sea_orm::DbErr> {
    let mut base_prices: HashMap<String, f64> = HashMap::new();

    for symbol in symbols {
        let norm_symbol = normalize_symbol(symbol).to_uppercase();

        // Try to get price at exact date, or closest date before
        let price = CoinsHistoricalPrices::find()
            .filter(coins_historical_prices::Column::Symbol.eq(&norm_symbol))
            .filter(coins_historical_prices::Column::Date.lte(date))
            .order_by(coins_historical_prices::Column::Date, Order::Desc)
            .one(db)
            .await?;

        if let Some(p) = price {
            if let Some(val) = p.price.to_f64() {
                base_prices.insert(norm_symbol, val);
            }
        }
    }

    Ok(base_prices)
}

/// Calculate ITP price from real-time constituent asset prices
/// Formula: ITP_price = initial_price * Σ(weight_i * current_price_i / base_price_i)
async fn calculate_itp_price(
    model: itps::Model,
    realtime_prices: &HashMap<String, f64>,
    db: &DatabaseConnection,
) -> ItpListEntry {
    // Parse assets and weights
    let assets: Option<Vec<String>> = model.assets.clone().and_then(|json| {
        serde_json::from_value(json).ok()
    });
    let weights: Option<Vec<f64>> = model.weights.clone().and_then(|json| {
        serde_json::from_value(json).ok()
    });

    // Get initial price (in 18 decimals)
    let initial_price = model.initial_price.as_ref()
        .and_then(|p| p.to_f64())
        .map(|v| v / 1e18)
        .unwrap_or(1000.0);

    // Get ITP creation date for base prices
    let creation_date = model.created_at
        .map(|dt| dt.date_naive())
        .unwrap_or_else(|| Utc::now().date_naive());

    // Calculate current price from constituents
    let mut current_price: Option<f64> = None;

    if let (Some(assets_list), Some(weights_list)) = (&assets, &weights) {
        if assets_list.len() == weights_list.len() && !assets_list.is_empty() {
            // Get base prices at creation date
            let base_prices = fetch_base_prices_at_date(db, assets_list, creation_date).await.unwrap_or_default();

            let mut weighted_performance = 0.0;
            let mut valid_count = 0;

            for (asset, weight) in assets_list.iter().zip(weights_list.iter()) {
                let norm_symbol = normalize_symbol(asset).to_uppercase();

                // Use real-time price from exchanges
                let current = realtime_prices.get(&norm_symbol);
                // Use historical base price, or fall back to current price if no history
                let base = base_prices.get(&norm_symbol).or(current);

                match (current, base) {
                    (Some(&curr), Some(&base_p)) if base_p > 0.0 => {
                        weighted_performance += weight * (curr / base_p);
                        valid_count += 1;
                        debug!("{}: realtime={:.2}, base={:.2}, weight={}, perf={:.4}",
                            norm_symbol, curr, base_p, weight, curr / base_p);
                    }
                    _ => {
                        // Use weight of 1.0 for missing assets (no change)
                        weighted_performance += weight * 1.0;
                        warn!("Missing price for {}, using neutral weight", norm_symbol);
                    }
                }
            }

            if valid_count > 0 {
                current_price = Some(initial_price * weighted_performance);
                debug!("ITP {} price: initial={:.2} * perf={:.4} = {:.2}",
                    model.symbol, initial_price, weighted_performance, current_price.unwrap_or(0.0));
            }
        }
    }

    // Fallback to initial_price if calculation failed
    if current_price.is_none() {
        current_price = Some(initial_price);
    }

    // Calculate AUM
    let total_supply_f64 = model.total_supply.as_ref()
        .and_then(|s| s.to_f64())
        .unwrap_or(0.0);

    let total_supply = model.total_supply
        .as_ref()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "0".to_string());

    let aum = current_price.map(|price| {
        (total_supply_f64 / 1e18) * price
    });

    let created_at = model.created_at
        .map(|dt| dt.timestamp())
        .unwrap_or(0);

    let initial_price_str = model.initial_price
        .as_ref()
        .map(|p| p.to_string());

    ItpListEntry {
        id: model.id,
        name: model.name,
        symbol: model.symbol,
        orbit_address: model.orbit_address,
        arbitrum_address: model.arbitrum_address,
        index_id: model.index_id,
        current_price,
        price_24h_change: None,
        initial_price: initial_price_str,
        total_supply,
        methodology: model.methodology,
        description: model.description,
        assets,
        weights,
        aum,
        admin_address: model.admin_address, // Story 2-3 AC#6
        created_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_creation() {
        let _service = ItpListingService::new();
    }

    #[test]
    fn test_normalize_symbol() {
        assert_eq!(normalize_symbol("BTC"), "BTC");
        assert_eq!(normalize_symbol("bitcoin"), "BTC");
        assert_eq!(normalize_symbol("ETH"), "ETH");
        assert_eq!(normalize_symbol("SOL"), "SOL");
    }
}
