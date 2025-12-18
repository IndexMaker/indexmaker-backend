use chrono::{Duration, NaiveDate, Utc};
use rust_decimal::Decimal;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, Order,
    QueryFilter, QueryOrder, QuerySelect, Set,
};
use std::collections::HashMap;
use tokio::time::{interval, Duration as TokioDuration};

use crate::entities::{daily_prices, rebalances, prelude::*};
use crate::services::coingecko::CoinGeckoService;
use crate::services::price_utils::get_or_fetch_coins_historical_price;
use crate::services::rebalancing::CoinRebalanceInfo;

pub async fn start_index_daily_prices_sync_job(
    db: DatabaseConnection,
    coingecko: CoinGeckoService,
) {
    tokio::spawn(async move {
        let mut interval = interval(TokioDuration::from_secs(86400)); // Every 24 hours

        loop {
            interval.tick().await;
            tracing::info!("Starting scheduled index daily prices sync");

            if let Err(e) = sync_index_daily_prices(&db, &coingecko).await {
                tracing::error!("Failed to sync index daily prices: {}", e);
            }
        }
    });
}

async fn sync_index_daily_prices(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Get all indexes
    let indexes = IndexMetadata::find().all(db).await?;

    if indexes.is_empty() {
        tracing::info!("No indexes found, skipping daily prices sync");
        return Ok(());
    }

    tracing::info!("Syncing daily prices for {} indexes", indexes.len());

    let today = Utc::now().date_naive();

    for index in indexes {
        // Get last stored date for this index
        let last_date = DailyPrices::find()
            .filter(daily_prices::Column::IndexId.eq(index.index_id.to_string()))
            .order_by(daily_prices::Column::Date, Order::Desc)
            .limit(1)
            .one(db)
            .await?
            .map(|row| row.date);

        let start_date = match last_date {
            Some(date) => date + Duration::days(1),
            None => {
                tracing::debug!(
                    "No existing prices for index {}, may still be backfilling. Using today.",
                    index.index_id
                );
                today
            }
        };

        if start_date > today {
            tracing::debug!(
                "Index {} is up to date (last date: {:?})",
                index.index_id,
                last_date
            );
            continue;
        }

        tracing::info!(
            "Syncing index {} from {} to {} ({} days)",
            index.index_id,
            start_date,
            today,
            (today - start_date).num_days() + 1
        );

        // Fill each missing day
        let mut date = start_date;
        let mut processed = 0;

        while date <= today {
            match calculate_and_store_index_price(db, coingecko, index.index_id, date).await {
                Ok(price) => {
                    tracing::info!(
                        "Stored daily price for index {} ({}): {} on {}",
                        index.index_id,
                        index.symbol,
                        price,
                        date
                    );
                    processed += 1;
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to calculate price for index {} on {}: {}",
                        index.index_id,
                        date,
                        e
                    );
                    // Continue with next date instead of failing entire sync
                }
            }

            date = date + Duration::days(1);
        }

        tracing::info!(
            "Processed {} days for index {}",
            processed,
            index.index_id
        );
    }

    tracing::info!("Index daily prices sync complete");
    Ok(())
}

/// Calculate index price for a specific date and store in daily_prices
async fn calculate_and_store_index_price(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
    index_id: i32,
    target_date: NaiveDate,
) -> Result<Decimal, Box<dyn std::error::Error + Send + Sync>> {
    // Check if price already exists for this date
    let existing = DailyPrices::find()
        .filter(daily_prices::Column::IndexId.eq(index_id.to_string()))
        .filter(daily_prices::Column::Date.eq(target_date))
        .one(db)
        .await?;

    if let Some(existing_price) = existing {
        tracing::debug!(
            "Price already exists for index {} on {}: {}",
            index_id,
            target_date,
            existing_price.price
        );
        return Ok(existing_price.price);
    }

    // Get the latest rebalance before or on target_date
    let target_timestamp = target_date
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc()
        .timestamp();

    let rebalance = Rebalances::find()
        .filter(rebalances::Column::IndexId.eq(index_id))
        .filter(rebalances::Column::Timestamp.lte(target_timestamp))
        .order_by(rebalances::Column::Timestamp, Order::Desc)
        .limit(1)
        .one(db)
        .await?
        .ok_or(format!(
            "No rebalance found for index {} before {}",
            index_id, target_date
        ))?;

    // Parse coins from rebalance
    let coins: Vec<CoinRebalanceInfo> = serde_json::from_value(rebalance.coins)?;

    if coins.is_empty() {
        return Err("Rebalance has no coins".into());
    }

    // Calculate index price: sum of (weight * quantity * token_price)
    let mut index_price = Decimal::ZERO;
    let mut quantities_map: HashMap<String, f64> = HashMap::new();
    let mut missing_prices = Vec::new();

    for coin in &coins {
        // Use self-healing price fetcher
        let token_price_result = get_or_fetch_coins_historical_price(
            db,
            coingecko,
            &coin.coin_id,
            &coin.symbol,
            target_date,
        )
        .await;

        match token_price_result {
            Ok(price) => {
                let weight: Decimal = coin.weight.parse()?;
                let quantity: Decimal = coin.quantity.parse()?;
                let price_decimal = Decimal::from_f64_retain(price)
                    .ok_or("Invalid price conversion")?;

                // index_price += weight * quantity * price
                index_price += weight * quantity * price_decimal;

                // Store quantity for the quantities field
                quantities_map.insert(coin.coin_id.clone(), quantity.to_string().parse()?);

                tracing::debug!(
                    "  {} ({}): weight={}, qty={}, price={}, contribution={}",
                    coin.symbol,
                    coin.coin_id,
                    weight,
                    quantity,
                    price,
                    weight * quantity * price_decimal
                );
            }
            Err(e) => {
                missing_prices.push(coin.coin_id.clone());
                tracing::warn!(
                    "Failed to get price for {} ({}) on {}: {}",
                    coin.symbol,
                    coin.coin_id,
                    target_date,
                    e
                );
            }
        }
    }

    if !missing_prices.is_empty() {
        return Err(format!(
            "Missing prices for {} tokens: {}",
            missing_prices.len(),
            missing_prices.join(", ")
        )
        .into());
    }

    // Store in daily_prices
    let quantities_json = serde_json::to_value(&quantities_map)?;

    let new_price = daily_prices::ActiveModel {
        index_id: Set(index_id.to_string()),
        date: Set(target_date),
        price: Set(index_price),
        quantities: Set(Some(quantities_json)),
        created_at: Set(Some(Utc::now().naive_utc())),
        updated_at: Set(Some(Utc::now().naive_utc())),
    };

    new_price.insert(db).await?;

    tracing::info!(
        "Calculated index price for index {} on {}: {} (from {} tokens)",
        index_id,
        target_date,
        index_price,
        coins.len()
    );

    Ok(index_price)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_price_calculation_logic() {
        let weight = Decimal::from_f64_retain(1.5).unwrap();
        
        let token_a = weight * Decimal::from(10) * Decimal::from(100);
        let token_b = weight * Decimal::from(20) * Decimal::from(50);
        let token_c = weight * Decimal::from(5) * Decimal::from(200);
        
        let total = token_a + token_b + token_c;
        
        assert_eq!(total, Decimal::from(4500));
    }
}