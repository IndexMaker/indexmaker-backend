use chrono::NaiveDate;
use rust_decimal::Decimal;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, Order,
    QueryFilter, QueryOrder, QuerySelect, Set,
};
use std::collections::HashMap;

use crate::entities::{daily_prices, rebalances, prelude::*};
use crate::services::coingecko::CoinGeckoService;
use crate::services::price_utils::get_or_fetch_coins_historical_price;
use crate::services::rebalancing::CoinRebalanceInfo;

/// Backfill daily prices for an index from initial_date to yesterday
pub async fn backfill_daily_prices(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
    index_id: i32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!("Starting daily prices backfill for index {}", index_id);

    // Get all rebalances for this index
    let rebalances = Rebalances::find()
        .filter(rebalances::Column::IndexId.eq(index_id))
        .order_by(rebalances::Column::Timestamp, Order::Asc)
        .all(db)
        .await?;

    if rebalances.is_empty() {
        tracing::info!("No rebalances found for index {}, skipping daily prices backfill", index_id);
        return Ok(());
    }

    tracing::info!(
        "Found {} rebalances for index {}, calculating daily prices",
        rebalances.len(),
        index_id
    );

    let today = chrono::Utc::now().date_naive();

    // Loop through rebalance periods
    for i in 0..rebalances.len() {
        let current_rebalance = &rebalances[i];
        
        // Start date: day after current rebalance
        let rebalance_date = chrono::DateTime::from_timestamp(current_rebalance.timestamp, 0)
            .unwrap()
            .date_naive();
        let start_date = rebalance_date + chrono::Duration::days(1);

        // End date: next rebalance date (or today if last rebalance)
        let end_date = if i + 1 < rebalances.len() {
            chrono::DateTime::from_timestamp(rebalances[i + 1].timestamp, 0)
                .unwrap()
                .date_naive()
        } else {
            today
        };

        if start_date > end_date {
            tracing::debug!(
                "Skipping period {}: start_date {} > end_date {}",
                i,
                start_date,
                end_date
            );
            continue;
        }

        tracing::info!(
            "Processing rebalance period {}/{}: {} to {} ({} days)",
            i + 1,
            rebalances.len(),
            start_date,
            end_date,
            (end_date - start_date).num_days() + 1
        );

        // Parse coins from this rebalance
        let coins: Vec<CoinRebalanceInfo> = serde_json::from_value(current_rebalance.coins.clone())?;

        // Fill prices for each day in this period
        let mut date = start_date;
        let mut processed = 0;
        let mut skipped = 0;

        while date <= end_date {
            match calculate_and_store_index_price(db, coingecko, index_id, date, &coins).await {
                Ok(true) => processed += 1,
                Ok(false) => skipped += 1,
                Err(e) => {
                    tracing::error!(
                        "Failed to calculate price for index {} on {}: {}",
                        index_id,
                        date,
                        e
                    );
                    // Continue with next date instead of failing entire backfill
                }
            }

            date = date + chrono::Duration::days(1);

            // Add small delay every 10 dates to avoid overwhelming the system
            if processed % 10 == 0 {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        }

        tracing::info!(
            "Completed period {}/{}: processed {} days, skipped {} (already existed)",
            i + 1,
            rebalances.len(),
            processed,
            skipped
        );
    }

    tracing::info!("Daily prices backfill complete for index {}", index_id);
    Ok(())
}

/// Calculate index price for a specific date and store in daily_prices
/// Returns Ok(true) if inserted, Ok(false) if already exists
async fn calculate_and_store_index_price(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
    index_id: i32,
    target_date: NaiveDate,
    coins: &[CoinRebalanceInfo],
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    // Check if price already exists for this date
    let existing = DailyPrices::find()
        .filter(daily_prices::Column::IndexId.eq(index_id.to_string()))
        .filter(daily_prices::Column::Date.eq(target_date))
        .one(db)
        .await?;

    if existing.is_some() {
        tracing::debug!(
            "Price already exists for index {} on {}, skipping",
            index_id,
            target_date
        );
        return Ok(false);
    }

    if coins.is_empty() {
        return Err("Rebalance has no coins".into());
    }

    // Calculate index price: sum of (weight * quantity * token_price)
    let mut index_price = Decimal::ZERO;
    let mut quantities_map: HashMap<String, f64> = HashMap::new();
    let mut missing_prices = Vec::new();

    for coin in coins {
        // Use self-healing price fetcher (same as rebalancing)
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

                tracing::trace!(
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
        created_at: Set(Some(chrono::Utc::now().naive_utc())),
        updated_at: Set(Some(chrono::Utc::now().naive_utc())),
    };

    new_price.insert(db).await?;

    tracing::debug!(
        "Stored index price for index {} on {}: {}",
        index_id,
        target_date,
        index_price
    );

    Ok(true)
}