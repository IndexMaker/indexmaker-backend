use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, IntoActiveModel, QueryFilter, Set};
use tokio::time::{interval, Duration};

use crate::entities::{coingecko_categories, prelude::*};
use crate::services::coingecko::CoinGeckoService;
use crate::services::sync_status::{self, jobs, intervals};

pub async fn start_category_sync_job(
    db: DatabaseConnection,
    coingecko: CoinGeckoService,
) {
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(86400)); // 24 hours

        // Check if we should run on startup based on last sync time
        match sync_status::should_sync(&db, jobs::CATEGORY_SYNC, intervals::CATEGORY_SYNC).await {
            Ok(true) => {
                tracing::info!("Starting category sync (startup or interval elapsed)");
                match sync_categories(&db, &coingecko).await {
                    Ok(_) => {
                        if let Err(e) = sync_status::record_success(&db, jobs::CATEGORY_SYNC, intervals::CATEGORY_SYNC).await {
                            tracing::warn!("Failed to record sync success: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to sync categories on startup: {}", e);
                        if let Err(e2) = sync_status::record_failure(&db, jobs::CATEGORY_SYNC, &e.to_string(), intervals::CATEGORY_SYNC).await {
                            tracing::warn!("Failed to record sync failure: {}", e2);
                        }
                    }
                }
            }
            Ok(false) => {
                tracing::info!("Skipping category sync on startup (recently synced)");
            }
            Err(e) => {
                tracing::warn!("Failed to check sync status, running sync anyway: {}", e);
                if let Err(e) = sync_categories(&db, &coingecko).await {
                    tracing::error!("Failed to sync categories: {}", e);
                }
            }
        }

        loop {
            interval.tick().await;

            // Check if enough time has passed since last sync
            match sync_status::should_sync(&db, jobs::CATEGORY_SYNC, intervals::CATEGORY_SYNC).await {
                Ok(true) => {
                    tracing::info!("Starting scheduled CoinGecko categories sync");
                    match sync_categories(&db, &coingecko).await {
                        Ok(_) => {
                            if let Err(e) = sync_status::record_success(&db, jobs::CATEGORY_SYNC, intervals::CATEGORY_SYNC).await {
                                tracing::warn!("Failed to record sync success: {}", e);
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to sync categories: {}", e);
                            if let Err(e2) = sync_status::record_failure(&db, jobs::CATEGORY_SYNC, &e.to_string(), intervals::CATEGORY_SYNC).await {
                                tracing::warn!("Failed to record sync failure: {}", e2);
                            }
                        }
                    }
                }
                Ok(false) => {
                    tracing::debug!("Skipping scheduled category sync (recently synced)");
                }
                Err(e) => {
                    tracing::warn!("Failed to check sync status: {}", e);
                }
            }
        }
    });
}

async fn sync_categories(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Fetch categories from CoinGecko
    let categories = coingecko.fetch_categories().await?;

    tracing::info!("Syncing {} categories to database", categories.len());

    let mut synced_count = 0;
    let mut updated_count = 0;

    for category in categories {
        // Check if category exists
        let existing = CoingeckoCategories::find()
            .filter(coingecko_categories::Column::CategoryId.eq(&category.category_id))
            .one(db)
            .await?;

        if let Some(existing_cat) = existing {
            // Update existing
            let mut active_model = existing_cat.into_active_model();
            active_model.name = Set(category.name);
            active_model.updated_at = Set(Some(Utc::now().naive_utc()));
            active_model.update(db).await?;
            updated_count += 1;
        } else {
            // Insert new
            let new_category = coingecko_categories::ActiveModel {
                category_id: Set(category.category_id),
                name: Set(category.name),
                updated_at: Set(Some(Utc::now().naive_utc())),
                ..Default::default()
            };
            new_category.insert(db).await?;
            synced_count += 1;
        }
    }

    tracing::info!(
        "Categories sync complete: {} new, {} updated",
        synced_count,
        updated_count
    );

    Ok(())
}
