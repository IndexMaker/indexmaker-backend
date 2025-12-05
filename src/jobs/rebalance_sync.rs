use chrono::Utc;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, Order, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
use tokio::time::{interval, Duration};

use crate::entities::{rebalances, prelude::*};
use crate::services::coingecko::CoinGeckoService;
use crate::services::rebalancing::{RebalancingService, RebalanceReason};

pub async fn start_rebalance_sync_job(
    db: DatabaseConnection,
    coingecko: CoinGeckoService,
) {
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(86400)); // Every day

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

        // Check if index has at least 1 constituent token
        use crate::entities::{index_constituents, prelude::*};
        let constituent_count = IndexConstituents::find()
            .filter(index_constituents::Column::IndexId.eq(index.index_id))
            .filter(index_constituents::Column::RemovedAt.is_null())
            .count(db)
            .await?;

        if constituent_count == 0 {
            tracing::debug!(
                "Skipping index {} - no constituent tokens configured",
                index.index_id
            );
            continue;
        }

        // Get last rebalance
        let last_rebalance = Rebalances::find()
            .filter(rebalances::Column::IndexId.eq(index.index_id))
            .order_by(rebalances::Column::Timestamp, Order::Desc)
            .limit(1)
            .one(db)
            .await?;

        // If no rebalances exist, trigger backfill
        if last_rebalance.is_none() {
            tracing::info!("No rebalances found for index {}, starting backfill", index.index_id);
            
            match rebalancing_service.backfill_historical_rebalances(index.index_id).await {
                Ok(_) => {
                    tracing::info!("Successfully completed backfill for index {}", index.index_id);
                }
                Err(e) => {
                    tracing::error!("Failed to backfill index {}: {}", index.index_id, e);
                }
            }
            
            // Continue to next index after backfill
            continue;
        }

        // Check if we need periodic rebalance
        let needs_rebalance = {
            let rb = last_rebalance.as_ref().unwrap();
            let time_since_last = current_time - rb.timestamp;
            let period_seconds = (rebalance_period as i64) * 86400;
            time_since_last >= period_seconds
        };

        if needs_rebalance {
            tracing::info!("Index {} needs periodic rebalancing", index.index_id);

            let current_date = Utc::now().date_naive();
            
            // Always Periodic here (Initial is handled by backfill)
            match rebalancing_service
                .perform_rebalance_for_date(index.index_id, current_date, RebalanceReason::Periodic)
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