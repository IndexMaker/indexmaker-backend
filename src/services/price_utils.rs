use chrono::{NaiveDate, Utc};
use rust_decimal::Decimal;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect};

use crate::{entities::{coins_historical_prices, prelude::*}, services::coingecko::CoinGeckoService};


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

/// Get historical price with automatic backfill from CoinGecko if missing.
///
/// This is a SELF-HEALING version that:
/// 1. Checks DB for exact date match
/// 2. If missing, fetches from CoinGecko (from last_stored_date+1 to target_date)
/// 3. Stores all fetched prices in DB
/// 4. Returns the target date price
///
/// This prevents rebalancing failures due to missing price data.
pub async fn get_or_fetch_coins_historical_price(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
    coin_id: &str,
    symbol: &str,
    target_date: NaiveDate,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    // Step 1: Try to get from DB first
    if let Some(price) = get_coins_historical_price_for_date(db, coin_id, target_date).await? {
        tracing::debug!("Found price for {} on {} in DB: ${}", symbol, target_date, price);
        return Ok(price);
    }

    // Step 2: Price missing - need to fetch from CoinGecko
    tracing::warn!(
        "Missing price for {} ({}) on {} - fetching from CoinGecko",
        symbol,
        coin_id,
        target_date
    );

    // Get last stored date for this coin
    let last_stored_date = get_last_stored_date_for_coin(db, coin_id).await?;

    // Calculate days to fetch
    let days_to_fetch = match last_stored_date {
        Some(last_date) if last_date < target_date => {
            // Fetch from last_date+1 to target_date
            let days = (target_date - last_date).num_days();
            tracing::info!(
                "Fetching {} days of prices for {} (from {} to {})",
                days,
                symbol,
                last_date.succ_opt().unwrap_or(last_date),
                target_date
            );
            days.to_string()
        }
        _ => {
            // No previous data or last_date >= target_date - fetch reasonable range
            let today = Utc::now().date_naive();
            let days = (today - target_date).num_days() + 1;
            tracing::info!(
                "Fetching {} days of prices for {} (no previous data)",
                days,
                symbol
            );
            days.to_string()
        }
    };

    // Step 3: Fetch from CoinGecko
    let stored_count = fetch_and_store_prices_for_coin(
        db,
        coingecko,
        coin_id,
        symbol,
        &days_to_fetch,
    )
    .await?;

    tracing::info!(
        "Stored {} new prices for {} ({})",
        stored_count,
        symbol,
        coin_id
    );

    // Step 4: Try to get the target date price again
    get_coins_historical_price_for_date(db, coin_id, target_date)
        .await?
        .ok_or_else(|| {
            format!(
                "Failed to fetch price for {} ({}) on {} from CoinGecko",
                symbol, coin_id, target_date
            )
            .into()
        })
}

/// Get the last stored date for a coin
async fn get_last_stored_date_for_coin(
    db: &DatabaseConnection,
    coin_id: &str,
) -> Result<Option<NaiveDate>, Box<dyn std::error::Error + Send + Sync>> {
    let last_record = CoinsHistoricalPrices::find()
        .filter(coins_historical_prices::Column::CoinId.eq(coin_id))
        .order_by(coins_historical_prices::Column::Date, Order::Desc)
        .limit(1)
        .one(db)
        .await?;

    Ok(last_record.map(|r| r.date))
}

/// Fetch historical prices from CoinGecko and store in database
/// (Reusable version of the sync job's fetch function)
async fn fetch_and_store_prices_for_coin(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
    coin_id: &str,
    symbol: &str,
    days: &str,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct MarketChartResponse {
        prices: Vec<[f64; 2]>,
        #[serde(default)]
        market_caps: Vec<[f64; 2]>,
        #[serde(default)]
        total_volumes: Vec<[f64; 2]>,
    }

    let url = format!("{}/coins/{}/market_chart", coingecko.base_url(), coin_id);

    let response = coingecko
        .client()
        .get(&url)
        .header("x-cg-pro-api-key", coingecko.api_key())
        .query(&[
            ("vs_currency", "usd"),
            ("days", days),
            ("interval", "daily"),
        ])
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_default();
        return Err(format!("CoinGecko API error {}: {}", status, error_text).into());
    }

    let data: MarketChartResponse = response.json().await?;

    if data.prices.is_empty() {
        return Ok(0);
    }

    let mut stored_count = 0;

    for i in 0..data.prices.len() {
        let timestamp_ms = data.prices[i][0] as i64;
        let price = data.prices[i][1];
        let market_cap = data.market_caps.get(i).map(|m| m[1]);
        let volume = data.total_volumes.get(i).map(|v| v[1]);

        let date = chrono::DateTime::from_timestamp_millis(timestamp_ms)
            .ok_or("Invalid timestamp")?
            .date_naive();

        // Check if already exists
        let exists = CoinsHistoricalPrices::find()
            .filter(coins_historical_prices::Column::CoinId.eq(coin_id))
            .filter(coins_historical_prices::Column::Date.eq(date))
            .one(db)
            .await?;

        if exists.is_some() {
            continue;
        }

        // Insert new record
        let new_price = coins_historical_prices::ActiveModel {
            coin_id: Set(coin_id.to_string()),
            symbol: Set(symbol.to_uppercase()),
            date: Set(date),
            price: Set(Decimal::from_f64_retain(price).ok_or("Invalid price")?),
            market_cap: Set(market_cap.and_then(Decimal::from_f64_retain)),
            volume: Set(volume.and_then(Decimal::from_f64_retain)),
            ..Default::default()
        };

        new_price.insert(db).await?;
        stored_count += 1;
    }

    Ok(stored_count)
}