use chrono::{DateTime, NaiveDate, Utc};
use dotenvy::dotenv;
use rust_decimal::Decimal;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Database, DatabaseConnection, EntityTrait, Order, QueryFilter,
    QueryOrder, QuerySelect, Set,
};
use serde::Deserialize;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use std::env;
use tokio::time::{sleep, Duration};

use indexmaker_backend::entities::{coins, coins_historical_prices, prelude::*};
use indexmaker_backend::services::coingecko::CoinGeckoService;

#[derive(Debug, Deserialize)]
struct MarketChartResponse {
    prices: Vec<[f64; 2]>,
    market_caps: Vec<[f64; 2]>,
    total_volumes: Vec<[f64; 2]>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load environment variables
    dotenv().ok();

    // Connect to database
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db = Database::connect(&database_url).await?;

    // Initialize CoinGecko service
    let coingecko_api_key = env::var("COINGECKO_API_KEY").expect("COINGECKO_API_KEY must be set");
    let coingecko_base_url = env::var("COINGECKO_BASE_URL")
        .unwrap_or_else(|_| "https://pro-api.coingecko.com/api/v3".to_string());

    let coingecko = CoinGeckoService::new(coingecko_api_key, coingecko_base_url);

    tracing::info!("Starting historical prices import...");

    // Get all active coins
    let all_coins = Coins::find()
        .filter(coins::Column::Active.eq(true))
        .all(&db)
        .await?;

    tracing::info!("Found {} active coins to process", all_coins.len());

    let mut success_count = 0;
    let mut skip_count = 0;
    let mut error_count = 0;

    for (index, coin) in all_coins.iter().enumerate() {
        let progress = index + 1;
        let total = all_coins.len();

        tracing::info!(
            "[{}/{}] Processing: {} ({})",
            progress,
            total,
            coin.symbol,
            coin.coin_id
        );

        // Check last stored date
        let last_date = get_last_stored_date(&db, &coin.coin_id).await?;

        let days_to_fetch = match last_date {
            Some(last) => {
                // Validate the date is reasonable (not epoch or future)
                let today = Utc::now().date_naive();

                if last < NaiveDate::from_ymd_opt(2019, 1, 1).unwrap() {
                    tracing::warn!("  Invalid last_date ({}), fetching ALL history", last);
                    "max".to_string()
                } else if last >= today {
                    // Already up to date
                    tracing::debug!("  {} is up to date (last: {}), skipping", coin.symbol, last);
                    skip_count += 1;
                    continue;
                } else {
                    let days_since = (today - last).num_days();

                    if days_since <= 1 {
                        tracing::debug!("  {} is up to date, skipping", coin.symbol);
                        skip_count += 1;
                        continue;
                    }

                    tracing::info!("  Fetching {} days since {}", days_since, last);
                    days_since.to_string()
                }
            }
            None => {
                tracing::info!("  No existing data, fetching ALL history (days=max)");
                "max".to_string()
            }
        };

        // Fetch from CoinGecko
        match fetch_and_store_prices(&db, &coingecko, &coin.coin_id, &coin.symbol, &days_to_fetch)
            .await
        {
            Ok(count) => {
                tracing::info!("  ✅ Stored {} price records", count);
                success_count += 1;
            }
            Err(e) => {
                tracing::error!("  ❌ Failed: {}", e);
                error_count += 1;
            }
        }

        // Rate limiting: 150ms between calls
        sleep(Duration::from_millis(150)).await;

        // Progress summary every 100 coins
        if progress % 100 == 0 {
            tracing::info!(
                "Progress: {}/{} coins | Success: {} | Skipped: {} | Errors: {}",
                progress,
                total,
                success_count,
                skip_count,
                error_count
            );
        }
    }

    tracing::info!("=== Import Complete ===");
    tracing::info!("Total coins: {}", all_coins.len());
    tracing::info!("Success: {}", success_count);
    tracing::info!("Skipped (up to date): {}", skip_count);
    tracing::info!("Errors: {}", error_count);

    Ok(())
}

/// Get the last stored date for a coin
async fn get_last_stored_date(
    db: &DatabaseConnection,
    coin_id: &str,
) -> Result<Option<NaiveDate>, Box<dyn std::error::Error>> {
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
) -> Result<usize, Box<dyn std::error::Error>> {
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
        return Err("No price data returned".into());
    }

    let mut stored_count = 0;

    for i in 0..data.prices.len() {
        let timestamp_ms = data.prices[i][0] as i64;
        let price = data.prices[i][1];
        let market_cap = data.market_caps.get(i).map(|m| m[1]);
        let volume = data.total_volumes.get(i).map(|v| v[1]);

        let date = DateTime::from_timestamp_millis(timestamp_ms)
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
            market_cap: Set(market_cap.and_then(|mc| Decimal::from_f64_retain(mc))),
            volume: Set(volume.and_then(|v| Decimal::from_f64_retain(v))),
            ..Default::default()
        };

        new_price.insert(db).await?;
        stored_count += 1;
    }

    Ok(stored_count)
}