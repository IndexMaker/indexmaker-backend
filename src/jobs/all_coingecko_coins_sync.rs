use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use tokio::time::{interval, Duration};

use crate::entities::{coins, prelude::*};
use crate::services::coingecko::CoinGeckoService;

pub async fn start_all_coingecko_coins_sync_job(
    db: DatabaseConnection,
    coingecko: CoinGeckoService,
) {
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(86400)); // Every 24 hours

        // Run immediately on startup
        tracing::info!("Running initial all CoinGecko coins sync");
        if let Err(e) = sync_all_coingecko_coins(&db, &coingecko).await {
            tracing::error!("Failed to sync all CoinGecko coins on startup: {}", e);
        }

        loop {
            interval.tick().await;
            tracing::info!("Starting scheduled all CoinGecko coins sync");

            if let Err(e) = sync_all_coingecko_coins(&db, &coingecko).await {
                tracing::error!("Failed to sync all CoinGecko coins: {}", e);
            }
        }
    });
}

async fn sync_all_coingecko_coins(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Check if coins table is empty
    let coin_count = Coins::find().count(db).await?;

    if coin_count == 0 {
        tracing::info!("Coins table is empty, fetching ALL coins from CoinGecko");
        sync_all_coins(db, coingecko).await?;
    } else {
        tracing::info!("Coins table has {} coins, fetching only new coins", coin_count);
        sync_new_coins(db, coingecko).await?;
    }

    Ok(())
}

/// Fetch and store ALL coins from CoinGecko (initial sync)
async fn sync_all_coins(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let all_coins = coingecko.fetch_all_coins_list().await?;

    tracing::info!("Fetched {} coins from CoinGecko /coins/list", all_coins.len());

    let mut inserted = 0;
    let mut updated = 0;

    for coin_info in all_coins {
        // Check if coin already exists
        let existing = Coins::find()
            .filter(coins::Column::CoinId.eq(&coin_info.id))
            .one(db)
            .await?;

        if let Some(existing_coin) = existing {
            // Update existing coin
            let mut active_model: coins::ActiveModel = existing_coin.into();
            active_model.symbol = Set(coin_info.symbol.clone());
            active_model.name = Set(coin_info.name.clone());
            active_model.platforms = Set(Some(serde_json::to_value(&coin_info.platforms)?));
            active_model.updated_at = Set(Some(Utc::now().naive_utc()));

            active_model.update(db).await?;
            updated += 1;
        } else {
            // Insert new coin
            let new_coin = coins::ActiveModel {
                coin_id: Set(coin_info.id.clone()),
                symbol: Set(coin_info.symbol.clone()),
                name: Set(coin_info.name.clone()),
                platforms: Set(Some(serde_json::to_value(&coin_info.platforms)?)),
                activated_at: Set(None), // Not available in /coins/list
                ..Default::default()
            };

            new_coin.insert(db).await?;
            inserted += 1;
        }
    }

    tracing::info!(
        "All coins sync complete: {} new, {} updated",
        inserted,
        updated
    );

    Ok(())
}

/// Fetch and store only NEW coins from CoinGecko (incremental sync)
async fn sync_new_coins(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let new_coins = coingecko.fetch_new_coins_list().await?;

    tracing::info!("Fetched {} new coins from CoinGecko /coins/list/new", new_coins.len());

    let mut inserted = 0;
    let mut skipped = 0;

    for coin_info in new_coins {
        // Check if coin already exists
        let existing = Coins::find()
            .filter(coins::Column::CoinId.eq(&coin_info.id))
            .one(db)
            .await?;

        if existing.is_some() {
            skipped += 1;
            continue;
        }

        // Insert new coin
        let new_coin = coins::ActiveModel {
            coin_id: Set(coin_info.id.clone()),
            symbol: Set(coin_info.symbol.clone()),
            name: Set(coin_info.name.clone()),
            platforms: Set(None), // Not available in /coins/list/new
            activated_at: Set(Some(coin_info.activated_at)),
            ..Default::default()
        };

        new_coin.insert(db).await?;
        inserted += 1;
    }

    tracing::info!(
        "New coins sync complete: {} new, {} skipped (already exist)",
        inserted,
        skipped
    );

    Ok(())
}