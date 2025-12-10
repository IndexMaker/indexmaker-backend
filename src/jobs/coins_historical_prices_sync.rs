use chrono::{NaiveDate, Utc};
use rust_decimal::Decimal;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter,
    QueryOrder, QuerySelect, Set,
};
use serde::Deserialize;
use tokio::time::{interval, Duration};

use crate::entities::{coins, coins_historical_prices, prelude::*};
use crate::services::coingecko::CoinGeckoService;

#[derive(Debug, Deserialize)]
struct MarketChartResponse {
    prices: Vec<[f64; 2]>,
    market_caps: Vec<[f64; 2]>,
    total_volumes: Vec<[f64; 2]>,
}

pub async fn start_coins_historical_prices_sync_job(
    db: DatabaseConnection,
    coingecko: CoinGeckoService,
) {
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(21600)); // Every 6 hours

        // Run immediately on startup
        tracing::info!("Running initial coins historical prices sync");
        if let Err(e) = sync_coins_historical_prices(&db, &coingecko).await {
            tracing::error!("Failed to sync coins historical prices on startup: {}", e);
        }

        loop {
            interval.tick().await;
            tracing::info!("Starting scheduled coins historical prices sync");

            if let Err(e) = sync_coins_historical_prices(&db, &coingecko).await {
                tracing::error!("Failed to sync coins historical prices: {}", e);
            }
        }
    });
}

async fn sync_coins_historical_prices(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let today = Utc::now().date_naive();

    // Get all active coins
    let all_coins = Coins::find()
        .filter(coins::Column::Active.eq(true))
        .all(db)
        .await?;

    tracing::info!("Checking {} active coins for price updates", all_coins.len());

    let mut fetched_count = 0;
    let mut up_to_date_count = 0;
    let mut error_count = 0;

    for coin in all_coins {
        // Check last stored date
        let last_date = get_last_stored_date(db, &coin.coin_id).await?;

        let needs_update = match last_date {
            Some(last) => {
                // Validate date is reasonable
                if last < NaiveDate::from_ymd_opt(2019, 1, 1).unwrap() {
                    tracing::warn!("Invalid last_date for {}, will fetch all history", coin.symbol);
                    true
                } else if last >= today {
                    // Already has today's data
                    false
                } else {
                    // Missing some days
                    true
                }
            }
            None => {
                // No data at all - new coin
                true
            }
        };

        if !needs_update {
            up_to_date_count += 1;
            continue;
        }

        let days_to_fetch = match last_date {
            Some(last) if last >= NaiveDate::from_ymd_opt(2019, 1, 1).unwrap() => {
                let days_since = (today - last).num_days();
                tracing::debug!("Fetching {} days for {} ({})", days_since, coin.symbol, coin.coin_id);
                days_since.to_string()
            }
            _ => {
                // New coin or invalid data - fetch all
                tracing::info!("Fetching ALL history for {} ({})", coin.symbol, coin.coin_id);
                "max".to_string()
            }
        };

        // Fetch and store prices
        match fetch_and_store_prices(db, coingecko, &coin.coin_id, &coin.symbol, &days_to_fetch)
            .await
        {
            Ok(count) => {
                if count > 0 {
                    tracing::debug!("Stored {} new prices for {}", count, coin.symbol);
                    fetched_count += 1;
                }
            }
            Err(e) => {
                tracing::warn!("Failed to fetch prices for {} ({}): {}", coin.symbol, coin.coin_id, e);
                error_count += 1;
            }
        }

        // Rate limiting: 150ms between calls
        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    tracing::info!(
        "Coins historical prices sync complete: {} updated, {} up-to-date, {} errors",
        fetched_count,
        up_to_date_count,
        error_count
    );

    Ok(())
}

/// Get the last stored date for a coin
async fn get_last_stored_date(
    db: &DatabaseConnection,
    coin_id: &str,
) -> Result<Option<NaiveDate>, Box<dyn std::error::Error + Send + Sync>> {
    let last_record = CoinsHistoricalPrices::find()
        .filter(coins_historical_prices::Column::CoinId.eq(coin_id))
        .order_by(coins_historical_prices::Column::Date, Order::Desc)
        .limit(1)
        .one(db)
        .await?;

    match last_record {
        Some(record) => {
            // Sanity check: date should be between 2019 and today
            let min_date = NaiveDate::from_ymd_opt(2019, 1, 1).unwrap();
            let max_date = Utc::now().date_naive();

            if record.date >= min_date && record.date <= max_date {
                Ok(Some(record.date))
            } else {
                tracing::warn!(
                    "Invalid date {} for coin {}, treating as no data",
                    record.date,
                    coin_id
                );
                Ok(None)
            }
        }
        None => Ok(None),
    }
}

/// Fetch historical prices from CoinGecko and store in database
async fn fetch_and_store_prices(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
    coin_id: &str,
    symbol: &str,
    days: &str,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
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
        return Ok(0); // No data, but not an error
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
            continue; // Skip duplicates
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