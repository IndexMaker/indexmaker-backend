/// THIS FILE WILL BE REMOVED SOON!


use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use tokio::time::{interval, Duration};

use crate::entities::{category_membership, prelude::*};
use crate::services::coingecko::{CoinGeckoService, CoinListItem};

const ALL_COINS_CATEGORY: &str = "all";

pub async fn start_all_coins_sync_job(
    db: DatabaseConnection,
    coingecko: CoinGeckoService,
) {
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(86400 * 7)); // Every 7 days

        // Run immediately on startup
        tracing::info!("Running initial all coins sync from /coins/list");
        if let Err(e) = sync_all_coins(&db, &coingecko).await {
            tracing::error!("Failed to sync all coins on startup: {}", e);
        }

        loop {
            interval.tick().await;
            tracing::info!("Starting scheduled all coins sync");

            if let Err(e) = sync_all_coins(&db, &coingecko).await {
                tracing::error!("Failed to sync all coins: {}", e);
            }
        }
    });
}

async fn sync_all_coins(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!("Fetching all coins from CoinGecko /coins/list endpoint");

    // Fetch all coins from /coins/list
    // let all_coins = coingecko.fetch_all_coins_list().await?;
    let all_coins: Vec<CoinListItem> = Vec::new();

    tracing::info!("Fetched {} total coins from CoinGecko", all_coins.len());

    let today = Utc::now().naive_utc();
    let mut added_count = 0;
    let mut updated_count = 0;
    let mut skipped_count = 0;

    for coin in all_coins {
        let coin_id = coin.id;
        let symbol = coin.symbol.to_uppercase();

        // Check if this coin already exists in category_membership with category_id="all"
        let existing = CategoryMembership::find()
            .filter(category_membership::Column::CoinId.eq(&coin_id))
            .filter(category_membership::Column::CategoryId.eq(ALL_COINS_CATEGORY))
            .one(db)
            .await?;

        if let Some(existing_record) = existing {
            // Update symbol if it changed or was null
            if existing_record.symbol.as_ref() != Some(&symbol) {
                use sea_orm::IntoActiveModel;
                let mut active_model = existing_record.into_active_model();
                active_model.symbol = Set(Some(symbol));
                active_model.updated_at = Set(Some(today));
                active_model.update(db).await?;
                updated_count += 1;
            } else {
                skipped_count += 1;
            }
        } else {
            // Insert new coin
            let new_membership = category_membership::ActiveModel {
                coin_id: Set(coin_id.clone()),
                category_id: Set(ALL_COINS_CATEGORY.to_string()),
                added_date: Set(today),
                removed_date: Set(None),
                symbol: Set(Some(symbol)),
                ..Default::default()
            };

            new_membership.insert(db).await?;
            added_count += 1;
        }
    }

    tracing::info!(
        "All coins sync complete: {} new, {} updated, {} skipped",
        added_count,
        updated_count,
        skipped_count
    );

    Ok(())
}