use chrono::{Duration, NaiveDate, Utc};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect};

use crate::{entities::{coins_historical_prices, category_membership, prelude::*}, services::coingecko::CoinGeckoService};


/// Get historical price for a coin on a specific date from coins_historical_prices table.
/// Uses coin_id (not symbol) for accurate lookups.
///
/// This is the NEW canonical implementation using the coins_historical_prices table.
/// The old get_historical_price_for_date() uses the historical_prices table and will be deprecated.
pub async fn get_coins_historical_price_for_date(
    db: &DatabaseConnection,
    coin_id: &str,
    target_date: NaiveDate,
) -> Result<Option<f64>, Box<dyn std::error::Error + Send + Sync>> {
    // Try exact match for target date
    let price_record = CoinsHistoricalPrices::find()
        .filter(coins_historical_prices::Column::CoinId.eq(coin_id))
        .filter(coins_historical_prices::Column::Date.eq(target_date))
        .one(db)
        .await?;

    if let Some(record) = price_record {
        return Ok(Some(record.price.to_string().parse::<f64>().unwrap_or(0.0)));
    }

    // No price found for this date
    tracing::debug!(
        "No price found for coin_id '{}' on {}",
        coin_id,
        target_date
    );

    Ok(None)
}



