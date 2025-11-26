use chrono::Utc;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect};
use tokio::time::{interval, Duration};

use crate::entities::{rebalances, prelude::*};
use crate::services::coingecko::CoinGeckoService;
use crate::services::rebalancing::{RebalancingService, RebalanceReason};

pub async fn start_rebalance_sync_job(
    db: DatabaseConnection,
    coingecko: CoinGeckoService,
) {
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(3600)); // Every hour

        let rebalancing_service = RebalancingService::new(db.clone(), coingecko);

        loop {
            interval.tick().await;
            tracing::info!("Starting scheduled rebalancing check");

            if let Err(e) = check_and_rebalance(&db, &rebalancing_service).await {
                tracing::error!("Failed to check and rebalance: {}", e);
            }
        }
    });
}

async fn check_and_rebalance(
    db: &DatabaseConnection,
    rebalancing_service: &RebalancingService,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Get all indexes
    let indexes = IndexMetadata::find().all(db).await?;

    let current_time = Utc::now().timestamp();

    for index in indexes {
        // Check if index has rebalancing configured
        let rebalance_period = match index.rebalance_period {
            Some(period) => period,
            None => continue, // Skip indexes without rebalance period
        };

        // Get last rebalance
        let last_rebalance = Rebalances::find()
            .filter(rebalances::Column::IndexId.eq(index.index_id))
            .order_by(rebalances::Column::Timestamp, Order::Desc)
            .limit(1)
            .one(db)
            .await?;

        // Check if we have a previous rebalance
        let is_first_rebalance = last_rebalance.is_none();

        let needs_rebalance = match &last_rebalance {
            Some(rb) => {
                let time_since_last = current_time - rb.timestamp;
                let period_seconds = (rebalance_period as i64) * 86400;
                time_since_last >= period_seconds
            }
            None => true, // No rebalance yet, create initial one
        };

        if needs_rebalance {
            tracing::info!("Index {} needs rebalancing", index.index_id);

            let current_date = Utc::now().date_naive();
            let reason = if is_first_rebalance {
                RebalanceReason::Initial
            } else {
                RebalanceReason::Periodic
            };

            match rebalancing_service
                .perform_rebalance_for_date(index.index_id, current_date, reason)
                .await
            {
                Ok(_) => tracing::info!("Successfully rebalanced index {}", index.index_id),
                Err(e) => tracing::error!("Failed to rebalance index {}: {}", index.index_id, e),
            }

            // Add delay before next index
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    Ok(())
}
