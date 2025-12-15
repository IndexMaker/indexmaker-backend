use chrono::{NaiveDate, Utc};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, Order, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
use tokio::time::{interval, Duration};

use crate::entities::{rebalances, prelude::*};
use crate::services::coingecko::CoinGeckoService;
use crate::services::exchange_api::ExchangeApiService;
use crate::services::rebalancing::{RebalancingService, RebalanceReason};

pub async fn start_rebalance_sync_job(
    db: DatabaseConnection,
    coingecko: CoinGeckoService,
    exchange_api: ExchangeApiService,
) {
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(86400)); // Every day

        let rebalancing_service = RebalancingService::new(
            db.clone(),
            coingecko,
            Some(exchange_api), // Pass exchange_api for scheduled rebalances
        );

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

        // Check if index has initial_date configured
        let initial_date = match index.initial_date {
            Some(date) => date,
            None => {
                tracing::warn!("Index {} has no initial_date, skipping", index.index_id);
                continue;
            }
        };

        // Calculate expected number of rebalances from initial_date to today
        let today = Utc::now().date_naive();
        let expected_rebalances = calculate_expected_rebalances(
            initial_date,
            rebalance_period,
            today,
        );

        // Count actual rebalances in database
        let actual_rebalances = Rebalances::find()
            .filter(rebalances::Column::IndexId.eq(index.index_id))
            .count(db)
            .await? as usize;


        // Check if backfill is incomplete
        if actual_rebalances < expected_rebalances {
            tracing::warn!(
                "Index {} has incomplete backfill: {} of {} rebalances. Starting backfill...",
                index.index_id,
                actual_rebalances,
                expected_rebalances
            );
            
            match rebalancing_service.backfill_historical_rebalances(index.index_id).await {
                Ok(_) => {
                    tracing::info!(
                        "Successfully completed backfill for index {} ({} rebalances)",
                        index.index_id,
                        expected_rebalances
                    );
                }
                Err(e) => {
                    tracing::error!("Failed to backfill index {}: {}", index.index_id, e);
                }
            }
            
            // Continue to next index after backfill
            continue;
        }

        // Get last rebalance
        let last_rebalance = Rebalances::find()
            .filter(rebalances::Column::IndexId.eq(index.index_id))
            .order_by(rebalances::Column::Timestamp, Order::Desc)
            .limit(1)
            .one(db)
            .await?;

        // Check if we need periodic rebalance
        let needs_rebalance = match last_rebalance {
            Some(rb) => {
                let time_since_last = current_time - rb.timestamp;
                let period_seconds = (rebalance_period as i64) * 86400;
                time_since_last >= period_seconds
            }
            None => {
                // This shouldn't happen since we verified count above, but handle it
                tracing::warn!("No last rebalance found for index {} despite count check", index.index_id);
                false
            }
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
                Err(e) => {
                    tracing::error!("Failed to rebalance index {}: {}", index.index_id, e);
                    tracing::error!("Skipping this rebalance cycle due to error. Will retry later.");
                }
            }

            // Add delay before next index
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    Ok(())
}

/// Calculate expected number of rebalances from initial_date to current_date
fn calculate_expected_rebalances(
    initial_date: NaiveDate,
    period_days: i32,
    current_date: NaiveDate,
) -> usize {
    if current_date < initial_date {
        return 0;
    }

    let total_days = (current_date - initial_date).num_days();
    let num_periods = (total_days / period_days as i64) as usize;
    
    // +1 for the initial rebalance
    num_periods + 1
}