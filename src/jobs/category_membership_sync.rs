use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, IntoActiveModel, QueryFilter, Set
};
use std::collections::HashSet;
use tokio::time::{interval, Duration};

use crate::entities::{category_membership, prelude::*};
use crate::services::coingecko::CoinGeckoService;

pub async fn start_category_membership_sync_job(
    db: DatabaseConnection,
    coingecko: CoinGeckoService,
) {
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(86400)); // Every 24 hours

        // Run immediately on startup to initialize
        tracing::info!("Running initial category membership sync");
        if let Err(e) = sync_category_membership(&db, &coingecko).await {
            tracing::error!("Failed to sync category membership on startup: {}", e);
        }

        loop {
            interval.tick().await;
            tracing::info!("Starting scheduled category membership sync");

            if let Err(e) = sync_category_membership(&db, &coingecko).await {
                tracing::error!("Failed to sync category membership: {}", e);
            }
        }
    });
}

async fn sync_category_membership(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Get all categories
    let categories = CoingeckoCategories::find().all(db).await?;

    let today = Utc::now().naive_utc();

    for category in categories {
        tracing::debug!("Syncing category: {}", category.category_id);

        // Fetch current tokens in this category from CoinGecko
        let current_coins = match coingecko.fetch_coins_by_category(&category.category_id).await {
            Ok(coins) => coins,
            Err(e) => {
                tracing::error!("Failed to fetch coins for category {}: {}", category.category_id, e);
                continue; // Skip this category and continue with others
            }
        };

        let current_tokens: HashSet<String> = current_coins
            .into_iter()
            .map(|c| c.id)
            .collect();

        // Get active tokens in category from database
        let active_memberships = CategoryMembership::find()
            .filter(category_membership::Column::CategoryId.eq(&category.category_id))
            .filter(category_membership::Column::RemovedDate.is_null())
            .all(db)
            .await?;

        let active_tokens: HashSet<String> = active_memberships
            .iter()
            .map(|m| m.coin_id.clone())
            .collect();

        let current_tokens_set: HashSet<String> = current_tokens.into_iter().collect();

        // Find new tokens (in current but not in active)
        let new_tokens: Vec<_> = current_tokens_set
            .difference(&active_tokens)
            .cloned()
            .collect();

        // Find removed tokens (in active but not in current)
        let removed_tokens: Vec<_> = active_tokens
            .difference(&current_tokens_set)
            .cloned()
            .collect();

        // Add new tokens
        for coin_id in new_tokens {
            let new_membership = category_membership::ActiveModel {
                coin_id: Set(coin_id.clone()),
                category_id: Set(category.category_id.clone()),
                added_date: Set(today),
                removed_date: Set(None),
                ..Default::default()
            };

            new_membership.insert(db).await?;
            tracing::info!("Added {} to category {}", coin_id, category.category_id);
        }

        // Mark removed tokens
        for coin_id in removed_tokens {
            // Find the active membership
            if let Some(membership) = active_memberships
                .iter()
                .find(|m| m.coin_id == coin_id)
            {
                let mut active_model = membership.clone().into_active_model();
                active_model.removed_date = Set(Some(today));
                active_model.updated_at = Set(Some(today));
                active_model.update(db).await?;
                
                tracing::info!("Removed {} from category {}", coin_id, category.category_id);
            }
        }
    }

    tracing::info!("Category membership sync complete");
    Ok(())
}

