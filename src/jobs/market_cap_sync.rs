use chrono::Utc;
use sea_orm::DatabaseConnection;
use std::sync::Arc;
use tokio::time::{interval, Duration};

use crate::services::market_cap::MarketCapService;

pub async fn start_market_cap_sync_job(
    db: DatabaseConnection,
    market_cap_service: Arc<MarketCapService>,
) {
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(86400)); // Every 24 hours

        // Run immediately on startup
        tracing::info!("Running initial market cap rankings sync");
        if let Err(e) = sync_market_cap_rankings(&db, &market_cap_service).await {
            tracing::error!("Failed to sync market cap on startup: {}", e);
        }

        loop {
            interval.tick().await;
            tracing::info!("Starting scheduled market cap rankings sync");

            if let Err(e) = sync_market_cap_rankings(&db, &market_cap_service).await {
                tracing::error!("Failed to sync market cap: {}", e);
            }
        }
    });
}

async fn sync_market_cap_rankings(
    db: &DatabaseConnection,
    market_cap_service: &MarketCapService,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let today = Utc::now().date_naive();

    tracing::info!("Fetching top 300 market cap rankings for {}", today);

    // Fetch top 300 for today
    let rankings = market_cap_service
        .get_top_tokens_by_market_cap(db, today, 300)
        .await?;

    tracing::info!("Fetched {} rankings, storing in database", rankings.len());

    // Store in database
    market_cap_service
        .store_rankings_in_db(db, today, &rankings)
        .await?;

    tracing::info!("Market cap rankings sync complete for {}", today);

    Ok(())
}