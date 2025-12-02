use chrono::{Duration, NaiveDate};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect};

use crate::entities::{historical_prices, prelude::*};

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
        .filter(historical_prices::Column::CoinId.eq(coin_id))
        .filter(historical_prices::Column::Timestamp.eq(target_timestamp))
        .one(db)
        .await?;

    if let Some(price_row) = exact_match {
        return Ok(Some(price_row.price));
    }

    // Fall back to nearest price within ±3 days
    let start_timestamp = (target_date - Duration::days(3))
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let end_timestamp = (target_date + Duration::days(3))
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc()
        .timestamp();

    let nearest = HistoricalPrices::find()
        .filter(historical_prices::Column::CoinId.eq(coin_id))
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

    Ok(None)
}
