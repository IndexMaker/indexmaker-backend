// src/scrapers/coin_resolver.rs

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect};
use crate::entities::{coins_historical_prices, prelude::*};

/// Resolve symbol to CoinGecko coin_id using highest market cap
pub async fn resolve_symbol_to_coin_id(
    db: &DatabaseConnection,
    symbol: &str,
) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
    // Use most recent data (today or last 7 days)
    let lookup_date = chrono::Utc::now().date_naive();

    // Try today's date first
    let coin = CoinsHistoricalPrices::find()
        .filter(coins_historical_prices::Column::Symbol.eq(symbol.to_uppercase()))
        .filter(coins_historical_prices::Column::Date.eq(lookup_date))
        .order_by(coins_historical_prices::Column::MarketCap, Order::Desc)
        .limit(1)
        .one(db)
        .await?;

    if let Some(c) = coin {
        return Ok(Some(c.coin_id));
    }

    // Fallback: Try most recent data within last 7 days
    let start_date = lookup_date - chrono::Duration::days(7);

    let coin = CoinsHistoricalPrices::find()
        .filter(coins_historical_prices::Column::Symbol.eq(symbol.to_uppercase()))
        .filter(coins_historical_prices::Column::Date.gte(start_date))
        .filter(coins_historical_prices::Column::Date.lte(lookup_date))
        .order_by(coins_historical_prices::Column::MarketCap, Order::Desc)
        .limit(1)
        .one(db)
        .await?;

    Ok(coin.map(|c| c.coin_id))
}