use std::env;
use sea_orm::Database;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use indexmaker_backend::services::coingecko::CoinGeckoService;
use indexmaker_backend::services::rebalancing::RebalancingService;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,indexmaker_backend=debug,sqlx=warn".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load environment variables
    dotenvy::dotenv().ok();

    // Get index_id from command line args
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: cargo run --bin backfill_rebalances <index_id>");
        eprintln!("Example: cargo run --bin backfill_rebalances 21");
        std::process::exit(1);
    }

    let index_id: i32 = args[1].parse().map_err(|_| {
        eprintln!("Invalid index_id. Must be a number.");
        std::process::exit(1);
    })?;

    // Connect to database
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    tracing::info!("Connecting to database...");
    let db = Database::connect(&database_url)
        .await
        .expect("Failed to connect to database");

    // Initialize CoinGecko service
    let coingecko_api_key = env::var("COINGECKO_API_KEY")
        .expect("COINGECKO_API_KEY must be set");
    let coingecko_base_url = env::var("COINGECKO_BASE_URL")
        .unwrap_or_else(|_| "https://pro-api.coingecko.com/api/v3".to_string());
    
    let coingecko = CoinGeckoService::new(coingecko_api_key, coingecko_base_url);

    // Create rebalancing service (without exchange_api for backfill mode)
    let rebalancing_service = RebalancingService::new(
        db.clone(),
        coingecko,
        None, // No exchange API needed for historical backfill
    );

    tracing::info!("🚀 Starting rebalance backfill for index {}", index_id);
    
    // Run backfill
    match rebalancing_service.backfill_historical_rebalances(index_id).await {
        Ok(_) => {
            tracing::info!("✅ Successfully completed rebalance backfill for index {}", index_id);
            Ok(())
        }
        Err(e) => {
            tracing::error!("❌ Failed to backfill rebalances for index {}: {}", index_id, e);
            Err(e)
        }
    }
}
