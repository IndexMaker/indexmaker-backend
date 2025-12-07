use chrono::{Duration, NaiveDate, Utc};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect};

use crate::{entities::{category_membership, historical_prices, prelude::*}, services::coingecko::CoinGeckoService};

/// Get historical price for a coin on a specific date with fallback logic.
///
/// Logic:
/// 1. Try exact match for target date (timestamp at 00:00:00 UTC)
/// 2. If not found, search for nearest price within ±3 days
/// 3. If still not found, return None
///
/// This is the canonical implementation used across the codebase for consistent
/// price lookups with appropriate fallback behavior.
pub async fn get_historical_price_for_date(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
    coin_id: &str,
    target_date: NaiveDate,
) -> Result<Option<f64>, Box<dyn std::error::Error + Send + Sync>> {
    let target_timestamp = target_date
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();

    // Try exact match first
    let exact_match = HistoricalPrices::find()
        .filter(historical_prices::Column::Symbol.eq(coin_id))
        .filter(historical_prices::Column::Timestamp.eq(target_timestamp))
        .one(db)
        .await?;

    if let Some(price_row) = exact_match {
        return Ok(Some(price_row.price));
    }

    // Fall back to nearest price within ±3 days
    let start_timestamp = (target_date - Duration::days(7))
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let end_timestamp = (target_date + Duration::days(7))
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc()
        .timestamp();

    let nearest = HistoricalPrices::find()
        .filter(historical_prices::Column::Symbol.eq(coin_id))
        .filter(historical_prices::Column::Timestamp.gte(start_timestamp))
        .filter(historical_prices::Column::Timestamp.lte(end_timestamp))
        .order_by(historical_prices::Column::Timestamp, Order::Desc)
        .limit(1)
        .one(db)
        .await?;

    if let Some(price_row) = nearest {
        let days_diff = (price_row.timestamp as i64 - target_timestamp) / 86400;
        tracing::debug!(
            "Using nearest price for {} on {} (timestamp diff: {} days)",
            coin_id,
            target_date,
            days_diff
        );
        return Ok(Some(price_row.price));
    }

    // ========== NEW: CoinGecko fallback ==========
    tracing::warn!(
        "No cached price found for symbol '{}' on {}, fetching from CoinGecko",
        coin_id,
        target_date
    );

    // Lookup CoinGecko coin_id from category_membership
    let membership = CategoryMembership::find()
        .filter(category_membership::Column::Symbol.eq(coin_id.to_uppercase()))  // symbol is uppercase in DB
        .filter(category_membership::Column::RemovedDate.is_null())
        .limit(1)
        .one(db)
        .await?;

    let coingecko_coin_id = match membership {
        Some(member) => member.coin_id,  // e.g., "polkadot"
        None => {
            tracing::error!("Could not find CoinGecko coin_id for symbol '{}'", coin_id);
            return Ok(None);
        }
    };

    // Calculate days from target_date to now
    let today = Utc::now().date_naive();
    let days_ago = (today - target_date).num_days();

    if days_ago < 0 {
        tracing::error!("Cannot fetch future price for {} on {}", coin_id, target_date);
        return Ok(None);
    }

    // Fetch from CoinGecko
    let prices = coingecko
        .get_token_market_chart(&coingecko_coin_id, "usd", (days_ago + 1) as u32)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch from CoinGecko for {} ({}): {}", coin_id, coingecko_coin_id, e);
            e
        })?;

    // Find closest price to target date
    let mut closest_price: Option<(i64, f64)> = None;
    let mut min_diff = i64::MAX;

    for (timestamp_ms, price) in &prices {
        let timestamp_sec = timestamp_ms / 1000;
        let diff = (timestamp_sec - target_timestamp).abs();
        
        if diff < min_diff {
            min_diff = diff;
            closest_price = Some((*timestamp_ms, *price));
        }
    }

    match closest_price {
        Some((timestamp_ms, price)) => {
            let timestamp_sec = (timestamp_ms / 1000) as i32;
            
            // Store in database with symbol and coingecko_coin_id
            let new_price = historical_prices::ActiveModel {
                coin_id: Set(coingecko_coin_id.clone()),  // Store CoinGecko ID
                symbol: Set(coin_id.to_string()),         // Store symbol (dot, btc, etc)
                timestamp: Set(timestamp_sec),
                price: Set(price),
                ..Default::default()
            };

            match new_price.insert(db).await {
                Ok(_) => {
                    tracing::info!(
                        "Fetched and stored price for {} ({}) on {}: ${:.2}",
                        coin_id,
                        coingecko_coin_id,
                        target_date,
                        price
                    );
                }
                Err(e) => {
                    tracing::debug!("Could not store price (might be duplicate): {}", e);
                }
            }

            Ok(Some(price))
        }
        None => {
            tracing::error!("CoinGecko returned no prices for {}", coin_id);
            Ok(None)
        }
    }
}


