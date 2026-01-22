//! Sync job for fetching coin logos from CoinGecko
//!
//! This job fetches logo URLs from CoinGecko's /coins/markets endpoint
//! and stores them in the coins.logo_address column.
//! It only fetches logos for coins that don't have one yet (persisted across restarts).

use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use tokio::time::{interval, Duration};

use crate::entities::{coins, prelude::*};
use crate::services::coingecko::CoinGeckoService;
use crate::services::sync_status::{self, jobs, intervals};

/// Start the logo sync background job
pub async fn start_coins_logo_sync_job(
    db: DatabaseConnection,
    coingecko: CoinGeckoService,
) {
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(3600)); // Check every hour

        // Check if we should run on startup
        match sync_status::should_sync(&db, jobs::COINS_LOGO_SYNC, intervals::COINS_LOGO_SYNC).await {
            Ok(true) => {
                tracing::info!("Starting coins logo sync (startup or interval elapsed)");
                match sync_coins_logos(&db, &coingecko).await {
                    Ok(updated) => {
                        tracing::info!("Logo sync complete: {} logos updated", updated);
                        if let Err(e) = sync_status::record_success(&db, jobs::COINS_LOGO_SYNC, intervals::COINS_LOGO_SYNC).await {
                            tracing::warn!("Failed to record sync success: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to sync coin logos: {}", e);
                        if let Err(e2) = sync_status::record_failure(&db, jobs::COINS_LOGO_SYNC, &e.to_string(), intervals::COINS_LOGO_SYNC).await {
                            tracing::warn!("Failed to record sync failure: {}", e2);
                        }
                    }
                }
            }
            Ok(false) => {
                tracing::info!("Skipping coins logo sync on startup (recently synced)");
            }
            Err(e) => {
                tracing::warn!("Failed to check sync status, running sync anyway: {}", e);
                if let Err(e) = sync_coins_logos(&db, &coingecko).await {
                    tracing::error!("Failed to sync coin logos: {}", e);
                }
            }
        }

        // Periodic check loop
        loop {
            interval.tick().await;

            match sync_status::should_sync(&db, jobs::COINS_LOGO_SYNC, intervals::COINS_LOGO_SYNC).await {
                Ok(true) => {
                    tracing::info!("Starting scheduled coins logo sync");
                    match sync_coins_logos(&db, &coingecko).await {
                        Ok(updated) => {
                            tracing::info!("Scheduled logo sync complete: {} logos updated", updated);
                            if let Err(e) = sync_status::record_success(&db, jobs::COINS_LOGO_SYNC, intervals::COINS_LOGO_SYNC).await {
                                tracing::warn!("Failed to record sync success: {}", e);
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to sync coin logos: {}", e);
                            if let Err(e2) = sync_status::record_failure(&db, jobs::COINS_LOGO_SYNC, &e.to_string(), intervals::COINS_LOGO_SYNC).await {
                                tracing::warn!("Failed to record sync failure: {}", e2);
                            }
                        }
                    }
                }
                Ok(false) => {
                    tracing::debug!("Skipping scheduled coins logo sync (recently synced)");
                }
                Err(e) => {
                    tracing::warn!("Failed to check sync status: {}", e);
                }
            }
        }
    });
}

/// Sync logos for all coins that don't have one yet
async fn sync_coins_logos(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    // Find all active coins without a logo
    let coins_without_logos = Coins::find()
        .filter(coins::Column::Active.eq(true))
        .filter(coins::Column::LogoAddress.is_null())
        .all(db)
        .await?;

    if coins_without_logos.is_empty() {
        tracing::info!("All active coins already have logos");
        return Ok(0);
    }

    tracing::info!(
        "Found {} active coins without logos, fetching from CoinGecko",
        coins_without_logos.len()
    );

    let mut updated_count = 0;

    // Process in batches of 250 (CoinGecko limit per request)
    const BATCH_SIZE: usize = 250;

    for (batch_idx, chunk) in coins_without_logos.chunks(BATCH_SIZE).enumerate() {
        let coin_ids: Vec<String> = chunk.iter().map(|c| c.coin_id.clone()).collect();

        tracing::info!(
            "Fetching logos batch {}: {} coins",
            batch_idx + 1,
            coin_ids.len()
        );

        // Fetch market data from CoinGecko (includes image/logo)
        match coingecko.fetch_markets(&coin_ids).await {
            Ok(market_data) => {
                // Update each coin with its logo
                for data in market_data {
                    // Find the coin in our chunk
                    if let Some(coin) = chunk.iter().find(|c| c.coin_id == data.id) {
                        let mut active_model: coins::ActiveModel = coin.clone().into();
                        active_model.logo_address = Set(Some(data.image.clone()));
                        active_model.updated_at = Set(Some(Utc::now().naive_utc()));

                        if let Err(e) = active_model.update(db).await {
                            tracing::warn!("Failed to update logo for {}: {}", data.id, e);
                        } else {
                            updated_count += 1;
                            tracing::debug!("Updated logo for {}: {}", data.id, data.image);
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to fetch market data for batch {}: {}", batch_idx + 1, e);
                // Continue with next batch instead of failing completely
            }
        }

        // Rate limiting: wait 1 second between batches to avoid hitting CoinGecko limits
        if batch_idx < coins_without_logos.chunks(BATCH_SIZE).count() - 1 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    tracing::info!("Logo sync complete: {} coins updated", updated_count);

    Ok(updated_count)
}
