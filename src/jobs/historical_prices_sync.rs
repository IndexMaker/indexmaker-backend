use chrono::{Duration, NaiveDate, Utc};
use sea_orm::{ActiveModelTrait,
    ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect, Set};
use std::collections::{HashMap, HashSet};
use tokio::time::{interval, Duration as TokioDuration};

use crate::entities::{historical_prices, prelude::*};
use crate::services::coingecko::CoinGeckoService;

pub async fn start_historical_prices_sync_job(
    db: DatabaseConnection,
    coingecko: CoinGeckoService,
) {
    tokio::spawn(async move {
        let mut interval = interval(TokioDuration::from_secs(86400)); // Every 24 hours

        // Run immediately on startup
        tracing::info!("Running initial historical prices sync");
        if let Err(e) = sync_historical_prices(&db, &coingecko).await {
            tracing::error!("Failed to sync historical prices on startup: {}", e);
        }

        loop {
            interval.tick().await;
            tracing::info!("Starting scheduled historical prices sync");

            if let Err(e) = sync_historical_prices(&db, &coingecko).await {
                tracing::error!("Failed to sync historical prices: {}", e);
            }
        }
    });
}

async fn sync_historical_prices(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Get all active tokens (tokens that appear in rebalances)
    let active_tokens = get_active_tokens(db).await?;

    if active_tokens.is_empty() {
        tracing::info!("No active tokens found, skipping historical prices sync");
        return Ok(());
    }

    tracing::info!("Syncing historical prices for {} active tokens", active_tokens.len());

    for (coin_id, symbol) in active_tokens {
        // Get the last stored price date for this token
        let last_price = HistoricalPrices::find()
            .filter(historical_prices::Column::Symbol.eq(&coin_id))
            .order_by(historical_prices::Column::Timestamp, Order::Desc)
            .limit(1)
            .one(db)
            .await?;

        let start_date = match last_price {
            Some(price) => {
                // Start from day after last stored price
                let last_date = chrono::DateTime::from_timestamp(price.timestamp as i64, 0)
                    .unwrap()
                    .date_naive();
                last_date + Duration::days(1)
            }
            None => {
                // No prices stored, fetch from 2019-01-01 (or earliest index initial_date)
                NaiveDate::from_ymd_opt(2019, 1, 1).unwrap()
            }
        };

        let end_date = Utc::now().date_naive();

        if start_date >= end_date {
            tracing::debug!("Prices for {} are up to date", coin_id);
            continue;
        }

        // Fetch and store prices
        match fetch_and_store_prices(db, coingecko, &coin_id, &symbol, start_date, end_date).await {
            Ok(count) => {
                tracing::info!("Stored {} new prices for {}", count, coin_id);
            }
            Err(e) => {
                tracing::error!("Failed to fetch prices for {}: {}", coin_id, e);
                // Continue with next token instead of failing entire sync
            }
        }

        // Add delay to avoid rate limiting
        tokio::time::sleep(TokioDuration::from_millis(500)).await;
    }

    tracing::info!("Historical prices sync complete");
    Ok(())
}

/// Get all unique coin IDs from index_constituents by looking up symbols in category_membership
async fn get_active_tokens(
    db: &DatabaseConnection,
) -> Result<Vec<(String, String)>, Box<dyn std::error::Error + Send + Sync>> {
    use crate::entities::{category_membership, index_constituents, prelude::*};
    
    // Get all active constituents
    let constituents = IndexConstituents::find()
        .filter(index_constituents::Column::RemovedAt.is_null())
        .all(db)
        .await?;

    let mut active_tokens: HashMap<String, String> = HashMap::new(); // coin_id -> symbol
    let mut lookup_failures = Vec::new();

    for constituent in constituents {
        // Look up coin_id from category_membership using symbol
        let membership = CategoryMembership::find()
            .filter(category_membership::Column::Symbol.eq(&constituent.token_symbol))
            .filter(category_membership::Column::RemovedDate.is_null())
            .limit(1)
            .one(db)
            .await?;

        match membership {
            Some(member) => {
                // Store coin_id (from CoinGecko) -> symbol (from constituent.coin_id)
                active_tokens.insert(member.coin_id, constituent.coin_id);
            }
            None => {
                // Track failures for logging
                lookup_failures.push(constituent.token_symbol.clone());
            }
        }
    }

    if !lookup_failures.is_empty() {
        tracing::warn!(
            "Failed to find coin_id for {} symbols: {}",
            lookup_failures.len(),
            lookup_failures.join(", ")
        );
    }

    let tokens: Vec<(String, String)> = active_tokens.into_iter().collect();
    tracing::info!("Found {} active tokens (coin_ids) from index constituents", tokens.len());

    Ok(tokens)
}

/// Fetch historical prices from CoinGecko and store in database
pub async fn fetch_and_store_prices(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
    coingecko_coin_id: &str,
    symbol: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let days = (end_date - start_date).num_days();

    if days <= 0 {
        return Ok(0);
    }

    // CoinGecko API limits to 365 days per call
    let mut stored_count = 0;
    let mut current_start = start_date;

    while current_start < end_date {
        let days_to_fetch = std::cmp::min(365, (end_date - current_start).num_days());

        if days_to_fetch <= 0 {
            break;
        }

        tracing::debug!(
            "Fetching {} days of prices for {} (from {})",
            days_to_fetch,
            coingecko_coin_id,
            current_start
        );

        // Fetch from CoinGecko with retry
        let prices = match fetch_with_retry(coingecko, coingecko_coin_id, days_to_fetch as u32).await {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("Failed to fetch prices for {}: {}", coingecko_coin_id, e);
                break; // Stop trying this token
            }
        };

        // Store in database
        for (timestamp_ms, price) in prices {
            let timestamp_sec = timestamp_ms / 1000;

            // Check if already exists
            let existing = HistoricalPrices::find()
                .filter(historical_prices::Column::CoinId.eq(coingecko_coin_id))
                .filter(historical_prices::Column::Timestamp.eq(timestamp_sec))
                .one(db)
                .await?;

            if existing.is_some() {
                continue; // Skip duplicates
            }

            // Insert new price
            let new_price = historical_prices::ActiveModel {
                coin_id: Set(coingecko_coin_id.to_string()),
                symbol: Set(symbol.to_string()), // Use symbol from index_constituents.coin_id
                timestamp: Set(timestamp_sec as i32),
                price: Set(price),
                ..Default::default()
            };

            new_price.insert(db).await?;
            stored_count += 1;
        }

        // Move to next chunk
        current_start = current_start + Duration::days(days_to_fetch);
    }

    Ok(stored_count)
}

/// Fetch prices with exponential backoff retry
async fn fetch_with_retry(
    coingecko: &CoinGeckoService,
    coin_id: &str,
    days: u32,
) -> Result<Vec<(i64, f64)>, Box<dyn std::error::Error + Send + Sync>> {
    let max_retries = 5;
    let mut delay = TokioDuration::from_secs(1);

    for attempt in 0..max_retries {
        match coingecko.get_token_market_chart(coin_id, "usd", days).await {
            Ok(prices) => return Ok(prices),
            Err(e) => {
                if attempt == max_retries - 1 {
                    return Err(e);
                }

                tracing::warn!(
                    "Fetch attempt {} failed for {}: {}. Retrying in {:?}",
                    attempt + 1,
                    coin_id,
                    e,
                    delay
                );

                tokio::time::sleep(delay).await;
                delay *= 2; // Exponential backoff
            }
        }
    }

    Err("Max retries exceeded".into())
}