use chrono::{NaiveDate, Utc};
use rust_decimal::Decimal;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde::Deserialize;
use tokio::time::{interval, Duration};

use crate::entities::{coins, coins_historical_prices, crypto_listings, prelude::*};
use crate::services::sync_status::{self, jobs, intervals};

/// Bitget kline response
#[derive(Debug, Deserialize)]
struct BitgetKlineResponse {
    code: String,
    msg: String,
    data: Vec<BitgetKline>,
}

/// Bitget kline data [timestamp, open, high, low, close, volume, quote_volume, ?]
#[derive(Debug, Deserialize)]
struct BitgetKline(
    String, // timestamp
    String, // open
    String, // high
    String, // low
    String, // close
    String, // base volume
    String, // quote volume
);

/// Start the Bitget historical prices sync job
pub async fn start_bitget_historical_prices_sync_job(db: DatabaseConnection) {
    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();

        // Wait 30 seconds after startup before first run
        tokio::time::sleep(Duration::from_secs(30)).await;

        let mut interval = interval(Duration::from_secs(86400)); // Daily

        loop {
            // Check if we should sync based on last sync time
            match sync_status::should_sync(
                &db,
                jobs::BITGET_HISTORICAL_PRICES,
                intervals::BITGET_HISTORICAL_PRICES,
            )
            .await
            {
                Ok(true) => {
                    tracing::info!("Starting Bitget historical prices sync");
                    match sync_bitget_historical_prices(&db, &client).await {
                        Ok(_) => {
                            if let Err(e) = sync_status::record_success(
                                &db,
                                jobs::BITGET_HISTORICAL_PRICES,
                                intervals::BITGET_HISTORICAL_PRICES,
                            )
                            .await
                            {
                                tracing::warn!("Failed to record sync success: {}", e);
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to sync Bitget historical prices: {}", e);
                            if let Err(e2) = sync_status::record_failure(
                                &db,
                                jobs::BITGET_HISTORICAL_PRICES,
                                &e.to_string(),
                                intervals::BITGET_HISTORICAL_PRICES,
                            )
                            .await
                            {
                                tracing::warn!("Failed to record sync failure: {}", e2);
                            }
                        }
                    }
                }
                Ok(false) => {
                    tracing::debug!("Skipping Bitget historical prices sync (recently synced)");
                }
                Err(e) => {
                    tracing::warn!("Failed to check sync status: {}", e);
                }
            }

            interval.tick().await;
        }
    });
}

/// Sync historical prices from Bitget for all Bitget-listed coins
async fn sync_bitget_historical_prices(
    db: &DatabaseConnection,
    client: &reqwest::Client,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let today = Utc::now().date_naive();

    // Get all active Bitget listings
    let bitget_listings = CryptoListings::find()
        .filter(crypto_listings::Column::Exchange.eq("bitget"))
        .filter(crypto_listings::Column::Status.eq("active"))
        .all(db)
        .await?;

    tracing::info!("Found {} active Bitget listings", bitget_listings.len());

    // Get all coins (for symbol to coin_id mapping)
    let all_coins = Coins::find().filter(coins::Column::Active.eq(true)).all(db).await?;

    // Create symbol -> coin_id map
    let symbol_to_coin_id: std::collections::HashMap<String, String> = all_coins
        .iter()
        .map(|c| (c.symbol.to_uppercase(), c.coin_id.clone()))
        .collect();

    let mut synced_count = 0;
    let mut skipped_count = 0;
    let mut error_count = 0;

    for listing in bitget_listings {
        let symbol = listing.symbol.to_uppercase();

        // Find the coin_id for this symbol
        let coin_id = match symbol_to_coin_id.get(&symbol) {
            Some(id) => id.clone(),
            None => {
                tracing::debug!("No coin_id found for Bitget symbol {}", symbol);
                skipped_count += 1;
                continue;
            }
        };

        // Determine the start date (listing date or default)
        let start_date = listing
            .listing_date
            .map(|dt| dt.date())
            .unwrap_or_else(|| NaiveDate::from_ymd_opt(2023, 1, 1).unwrap());

        // Get the last synced date for this coin
        let last_date = get_last_synced_date(db, &coin_id).await?;

        // Calculate days to fetch
        let fetch_from = match last_date {
            Some(date) if date >= today => {
                // Already up to date
                skipped_count += 1;
                continue;
            }
            Some(date) => date + chrono::Duration::days(1),
            None => start_date,
        };

        // Construct trading pair (e.g., "BTCUSDT")
        let trading_pair = format!("{}USDT", symbol);

        // Fetch historical klines from Bitget
        match fetch_bitget_klines(
            client,
            &trading_pair,
            fetch_from,
            today,
        )
        .await
        {
            Ok(klines) => {
                if klines.is_empty() {
                    skipped_count += 1;
                    continue;
                }

                // Store in database
                let stored = store_historical_prices(db, &coin_id, &symbol, klines).await?;

                if stored > 0 {
                    tracing::debug!("Stored {} historical prices for {}", stored, symbol);
                    synced_count += 1;
                } else {
                    skipped_count += 1;
                }
            }
            Err(e) => {
                tracing::warn!("Failed to fetch Bitget klines for {}: {}", symbol, e);
                error_count += 1;
            }
        }

        // Rate limiting: 100ms between requests
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    tracing::info!(
        "✅ Bitget historical prices sync complete: {} synced, {} skipped, {} errors",
        synced_count,
        skipped_count,
        error_count
    );

    Ok(())
}

/// Get the last synced date for a coin
async fn get_last_synced_date(
    db: &DatabaseConnection,
    coin_id: &str,
) -> Result<Option<NaiveDate>, Box<dyn std::error::Error + Send + Sync>> {
    let result = CoinsHistoricalPrices::find()
        .filter(coins_historical_prices::Column::CoinId.eq(coin_id))
        .order_by_desc(coins_historical_prices::Column::Date)
        .one(db)
        .await?;

    Ok(result.map(|r| r.date))
}

/// Fetch historical klines from Bitget API
async fn fetch_bitget_klines(
    client: &reqwest::Client,
    symbol: &str, // e.g., "BTCUSDT"
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<(NaiveDate, Decimal, Option<Decimal>)>, Box<dyn std::error::Error + Send + Sync>> {
    let mut all_klines = Vec::new();
    let mut end_time = to
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc()
        .timestamp_millis();

    let from_timestamp = from
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis();

    // Bitget API: max 1000 candles per request, use pagination
    loop {
        let url = format!(
            "https://api.bitget.com/api/v2/spot/market/history-candles?symbol={}&granularity=1day&endTime={}&limit=1000",
            symbol, end_time
        );

        let response = match client.get(&url).send().await {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!("Bitget API request failed for {}: {}", symbol, e);
                break;
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            tracing::warn!("Bitget API error for {}: {} - {}", symbol, status, body);
            break;
        }

        let kline_response: BitgetKlineResponse = response.json().await?;

        if kline_response.code != "00000" {
            tracing::warn!("Bitget API error for {}: {}", symbol, kline_response.msg);
            break;
        }

        if kline_response.data.is_empty() {
            break;
        }

        for kline in &kline_response.data {
            let timestamp_ms: i64 = kline.0.parse()?;

            // Check if we've gone past the start date
            if timestamp_ms < from_timestamp {
                // Found all data we need
                return Ok(all_klines);
            }

            let date = match chrono::DateTime::from_timestamp_millis(timestamp_ms) {
                Some(dt) => dt.date_naive(),
                None => continue,
            };

            // Use close price
            let close_price: Decimal = kline.4.parse()?;
            let volume: Option<Decimal> = kline.6.parse().ok();

            all_klines.push((date, close_price, volume));
        }

        // Get oldest timestamp for pagination
        if let Some(oldest) = kline_response.data.last() {
            let oldest_ts: i64 = oldest.0.parse()?;
            if oldest_ts <= from_timestamp {
                break; // Reached start date
            }
            end_time = oldest_ts - 1; // Move pagination backward
        } else {
            break;
        }

        // Rate limiting between pagination requests
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    Ok(all_klines)
}

/// Store historical prices in database
async fn store_historical_prices(
    db: &DatabaseConnection,
    coin_id: &str,
    symbol: &str,
    prices: Vec<(NaiveDate, Decimal, Option<Decimal>)>,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let mut stored_count = 0;

    for (date, price, volume) in prices {
        // Check if already exists
        let exists = CoinsHistoricalPrices::find()
            .filter(coins_historical_prices::Column::CoinId.eq(coin_id))
            .filter(coins_historical_prices::Column::Date.eq(date))
            .one(db)
            .await?;

        if exists.is_some() {
            continue;
        }

        let new_price = coins_historical_prices::ActiveModel {
            coin_id: Set(coin_id.to_string()),
            symbol: Set(symbol.to_uppercase()),
            date: Set(date),
            price: Set(price),
            volume: Set(volume),
            market_cap: Set(None), // Bitget doesn't provide market cap
            ..Default::default()
        };

        new_price.insert(db).await?;
        stored_count += 1;
    }

    Ok(stored_count)
}
