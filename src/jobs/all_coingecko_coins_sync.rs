use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, Set};
use tokio::time::{interval, Duration};
use std::collections::HashMap;

use crate::entities::{coins, prelude::*};
use crate::services::coingecko::{CoinGeckoService, CoinListItem};

pub async fn start_all_coingecko_coins_sync_job(
    db: DatabaseConnection,
    coingecko: CoinGeckoService,
) {
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(86400)); // Every 24 hours

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
        sync_all_coins_initial(db, coingecko).await?;
    } else {
        tracing::info!("Coins table has {} coins, running incremental sync", coin_count);
        sync_all_coins_incremental(db, coingecko).await?;
    }

    Ok(())
}

/// Initial sync: Fetch ALL coins (active + inactive)
async fn sync_all_coins_initial(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Fetch active coins
    tracing::info!("Fetching active coins...");
    let active_coins = coingecko.fetch_all_coins_list("active").await?;
    tracing::info!("Fetched {} active coins", active_coins.len());

    // Fetch inactive coins
    tracing::info!("Fetching inactive coins...");
    let inactive_coins = coingecko.fetch_all_coins_list("inactive").await?;
    // let inactive_coins: Vec<CoinListItem> = Vec::new();
    tracing::info!("Fetched {} inactive coins", inactive_coins.len());

    // Combine and store
    let mut inserted = 0;

    for (coins_list, is_active) in [
        (active_coins, true),
        (inactive_coins, false),
    ] {
        for coin_info in coins_list {
            let new_coin = coins::ActiveModel {
                coin_id: Set(coin_info.id.clone()),
                symbol: Set(coin_info.symbol.clone()),
                name: Set(coin_info.name.clone()),
                platforms: Set(None), // Leave as NULL for now
                active: Set(is_active),
                activated_at: Set(None),
                ..Default::default()
            };

            new_coin.insert(db).await?;
            inserted += 1;
        }
    }

    tracing::info!("Initial sync complete: {} total coins inserted", inserted);

    Ok(())
}

/// Incremental sync: Update existing + add new coins
async fn sync_all_coins_incremental(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Fetch active coins
    tracing::info!("Fetching active coins...");
    let active_coins = coingecko.fetch_all_coins_list("active").await?;
    
    // Fetch inactive coins
    tracing::info!("Fetching inactive coins...");
    // let inactive_coins = coingecko.fetch_all_coins_list("inactive").await?;
    let inactive_coins: Vec<CoinListItem> = Vec::new();
    
    // Fetch newly added coins
    tracing::info!("Fetching newly added coins...");
    let new_coins = coingecko.fetch_new_coins_list().await?;

    // Build a map: coin_id -> active status
    let mut coin_active_map: HashMap<String, bool> = HashMap::new();
    
    for coin in &active_coins {
        coin_active_map.insert(coin.id.clone(), true);
    }
    
    for coin in &inactive_coins {
        coin_active_map.insert(coin.id.clone(), false);
    }

    // Build complete coins map for updates
    let mut all_coins_map: HashMap<String, _> = HashMap::new();
    
    for coin in active_coins {
        all_coins_map.insert(coin.id.clone(), coin);
    }
    
    for coin in inactive_coins {
        all_coins_map.insert(coin.id.clone(), coin);
    }

    let mut inserted = 0;
    let mut updated = 0;

    // Update existing coins
    let existing_coins = Coins::find().all(db).await?;
    
    for existing_coin in existing_coins {
        if let Some(coin_info) = all_coins_map.get(&existing_coin.coin_id) {
            let is_active = coin_active_map.get(&existing_coin.coin_id).copied().unwrap_or(true);

            // Check if anything changed
            let needs_update = existing_coin.symbol != coin_info.symbol
                || existing_coin.name != coin_info.name
                || existing_coin.active != is_active;

            if needs_update {
                let mut active_model: coins::ActiveModel = existing_coin.into();
                active_model.symbol = Set(coin_info.symbol.clone());
                active_model.name = Set(coin_info.name.clone());
                active_model.active = Set(is_active);
                active_model.updated_at = Set(Some(Utc::now().naive_utc()));

                active_model.update(db).await?;
                updated += 1;
            }
        }
    }

    // Insert newly added coins
    for new_coin_info in new_coins {
        let exists = Coins::find()
            .filter(coins::Column::CoinId.eq(&new_coin_info.id))
            .one(db)
            .await?;

        if exists.is_some() {
            continue;
        }

        let is_active = coin_active_map.get(&new_coin_info.id).copied().unwrap_or(true);

        let new_coin = coins::ActiveModel {
            coin_id: Set(new_coin_info.id.clone()),
            symbol: Set(new_coin_info.symbol.clone()),
            name: Set(new_coin_info.name.clone()),
            platforms: Set(None), // Leave as NULL for now
            active: Set(is_active),
            activated_at: Set(Some(new_coin_info.activated_at)),
            ..Default::default()
        };

        new_coin.insert(db).await?;
        inserted += 1;
    }

    tracing::info!(
        "Incremental sync complete: {} new, {} updated",
        inserted,
        updated
    );

    Ok(())
}