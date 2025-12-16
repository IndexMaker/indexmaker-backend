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

#[derive(Debug, Clone)]
struct CoinSyncInfo {
    coin_id: String,
    symbol: String,
    last_date: Option<NaiveDate>,
    market_cap: Option<Decimal>,
}

pub async fn start_coins_historical_prices_sync_job(
    db: DatabaseConnection,
    coingecko: CoinGeckoService,
) {
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(21600)); // Every 6 hours

        // // Run immediately on startup
        // tracing::info!("Running initial coins historical prices sync");
        // if let Err(e) = sync_coins_historical_prices(&db, &coingecko).await {
        //     tracing::error!("Failed to sync coins historical prices on startup: {}", e);
        // }

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

    tracing::info!("Found {} active coins", all_coins.len());

    // OPTIMIZATION: Get ALL last dates + market caps in ONE query
    let all_last_dates = get_all_coins_last_dates_batch(db).await?;

    tracing::info!(
        "Retrieved last dates for {} coins in single query",
        all_last_dates.len()
    );

    // Build map: coin_id -> (last_date, market_cap)
    let last_date_map: std::collections::HashMap<String, (Option<NaiveDate>, Option<Decimal>)> =
        all_last_dates
            .into_iter()
            .map(|info| (info.coin_id.clone(), (info.last_date, info.market_cap)))
            .collect();

    // Prepare sync info for all coins
    let all_coin_sync_info: Vec<CoinSyncInfo> = all_coins
        .into_iter()
        .map(|coin| {
            let (last_date, market_cap) = last_date_map
                .get(&coin.coin_id)
                .cloned()
                .unwrap_or((None, None));

            CoinSyncInfo {
                coin_id: coin.coin_id,
                symbol: coin.symbol,
                last_date,
                market_cap,
            }
        })
        .collect();

    // OPTIMIZATION: Filter to top 1000 by market cap + all new tokens (NULL market cap)
    let coins_to_sync = select_top_coins_to_sync(all_coin_sync_info, 1000);

    tracing::info!(
        "Selected {} coins to sync (top 1000 by market cap + new tokens)",
        coins_to_sync.len()
    );

    let mut fetched_count = 0;
    let mut up_to_date_count = 0;
    let mut error_count = 0;
    let mut new_token_count = 0;

    let total = coins_to_sync.len();
    
    for (index, coin_info) in coins_to_sync.iter().enumerate() {
        let progress = index + 1;

        // Check if needs update
        let needs_update = match coin_info.last_date {
            Some(last) => {
                // Validate date is reasonable
                if last < NaiveDate::from_ymd_opt(2019, 1, 1).unwrap() {
                    tracing::warn!("Invalid last_date for {}, will fetch all history", coin_info.symbol);
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
                // No data at all
                true
            }
        };

        if !needs_update {
            up_to_date_count += 1;
            continue;
        }

        // Calculate days to fetch
        let days_to_fetch = if coin_info.market_cap.is_none() {
            // New token (no market_cap): fetch all history
            new_token_count += 1;
            tracing::info!("New token detected: {} - fetching full history", coin_info.symbol);
            "max".to_string()
        } else {
            // Existing token: incremental update
            match coin_info.last_date {
                Some(last) if last >= NaiveDate::from_ymd_opt(2019, 1, 1).unwrap() => {
                    let days_since = (today - last).num_days();
                    days_since.to_string()
                }
                _ => {
                    // Has market_cap but no price data? Fetch all
                    "max".to_string()
                }
            }
        };

        // Fetch and store prices
        match fetch_and_store_prices(db, coingecko, &coin_info.coin_id, &coin_info.symbol, &days_to_fetch)
            .await
        {
            Ok(count) => {
                if count > 0 {
                    tracing::debug!("Stored {} new prices for {}", count, coin_info.symbol);
                    fetched_count += 1;
                }
            }
            Err(e) => {
                tracing::warn!("Failed to fetch prices for {} ({}): {}", coin_info.symbol, coin_info.coin_id, e);
                error_count += 1;
            }
        }

        // Rate limiting: 120ms between calls
        tokio::time::sleep(Duration::from_millis(120)).await;
        
        // Progress summary every 100 coins
        if progress % 100 == 0 {
            tracing::info!(
                "Progress: {}/{} coins | Success: {} | Errors: {}",
                progress,
                total,
                fetched_count,
                error_count
            );
        }
    }

    tracing::info!(
        "Coins historical prices sync complete: {} updated, {} up-to-date, {} new tokens, {} errors (total synced: {} coins)",
        fetched_count,
        up_to_date_count,
        new_token_count,
        error_count,
        coins_to_sync.len()
    );

    Ok(())
}

/// Get ALL coins' last dates + market caps in ONE batch query using DISTINCT ON
async fn get_all_coins_last_dates_batch(
    db: &DatabaseConnection,
) -> Result<Vec<CoinSyncInfo>, Box<dyn std::error::Error + Send + Sync>> {
    use sea_orm::FromQueryResult;

    #[derive(Debug, FromQueryResult)]
    struct LastDateRecord {
        coin_id: String,
        symbol: String,
        date: NaiveDate,
        market_cap: Option<Decimal>,
    }

    // Use raw SQL for PostgreSQL DISTINCT ON
    let records: Vec<LastDateRecord> = LastDateRecord::find_by_statement(
        sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            r#"
            SELECT DISTINCT ON (coin_id)
                coin_id,
                symbol,
                date,
                market_cap
            FROM coins_historical_prices
            ORDER BY coin_id, date DESC
            "#,
            vec![],
        ),
    )
    .all(db)
    .await?;

    let results = records
        .into_iter()
        .map(|r| CoinSyncInfo {
            coin_id: r.coin_id,
            symbol: r.symbol,
            last_date: Some(r.date),
            market_cap: r.market_cap,
        })
        .collect();

    Ok(results)
}

/// Select top N coins by market cap + all coins with NULL market cap (new tokens)
fn select_top_coins_to_sync(all_coins: Vec<CoinSyncInfo>, top_n: usize) -> Vec<CoinSyncInfo> {
    // Partition by market_cap existence
    let (with_mcap, without_mcap): (Vec<_>, Vec<_>) = all_coins
        .into_iter()
        .partition(|c| c.market_cap.is_some());

    // Sort coins WITH market_cap by market_cap DESC
    let mut sorted_by_mcap = with_mcap;
    sorted_by_mcap.sort_by(|a, b| {
        b.market_cap
            .unwrap_or(Decimal::ZERO)
            .cmp(&a.market_cap.unwrap_or(Decimal::ZERO))
    });

    // Take top N by market cap
    let mut result: Vec<_> = sorted_by_mcap.into_iter().take(top_n).collect();

    // Append all coins without market_cap (new tokens)
    let new_tokens_count = without_mcap.len();
    result.extend(without_mcap);

    tracing::info!(
        "Selected {} coins: {} top by market cap + {} new tokens",
        result.len(),
        top_n.min(result.len() - new_tokens_count),
        new_tokens_count
    );

    result
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