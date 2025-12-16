use axum::{routing::{get, post}, Router};
use sea_orm::{Database, DatabaseConnection};
use sea_orm_migration::MigratorTrait;
use std::env;
use tower_http::cors::{CorsLayer, Any};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod entities;
mod jobs;
mod handlers;
mod models;
mod scrapers;
mod services;

use jobs::{
    category_sync,
    rebalance_sync,
    category_membership_sync,
    announcement_scraper,
    index_daily_prices_sync,
};
use services::coingecko::CoinGeckoService;

use crate::{jobs::{all_coingecko_coins_sync, coins_historical_prices_sync}, scrapers::ScraperConfig, services::exchange_api::ExchangeApiService};

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub coingecko: CoinGeckoService,
    pub exchange_api: ExchangeApiService,
}

#[tokio::main]
async fn main() {
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

    // Connect to database
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    tracing::info!("Connecting to database...");
    let db = Database::connect(&database_url)
        .await
        .expect("Failed to connect to database");

    // Run migrations
    tracing::info!("Running migrations...");
    migration::Migrator::up(&db, None)
        .await
        .expect("Failed to run migrations");

    // Initialize CoinGecko service
    let coingecko_api_key = env::var("COINGECKO_API_KEY")
        .expect("COINGECKO_API_KEY must be set");
    let coingecko_base_url = env::var("COINGECKO_BASE_URL")
        .unwrap_or_else(|_| "https://pro-api.coingecko.com/api/v3".to_string());
    
    // Initialize scraper config
    let scraper_config = ScraperConfig {
        scrape_api_key: env::var("SCRAPER_API_KEY")
            .expect("SCRAPER_API_KEY must be set"),
        retry_max: 3,
        retry_delay_ms: 1000,
    };
    
    let coingecko = CoinGeckoService::new(coingecko_api_key, coingecko_base_url);

    // Initialize Exchange API service (10 minute cache)
    let exchange_api = ExchangeApiService::new(600);

    let state = AppState {
        db: db.clone(),
        coingecko: coingecko.clone(),
        exchange_api: exchange_api.clone(),
    };

    // Start background jobs
    // all_coingecko_coins_sync::start_all_coingecko_coins_sync_job(db.clone(), coingecko.clone()).await;
    coins_historical_prices_sync::start_coins_historical_prices_sync_job(db.clone(), coingecko.clone()).await;
    // category_sync::start_category_sync_job(db.clone(), coingecko.clone()).await;
    rebalance_sync::start_rebalance_sync_job(db.clone(), coingecko.clone(), exchange_api.clone()).await;
    // category_membership_sync::start_category_membership_sync_job(db.clone(), coingecko.clone()).await;
    // announcement_scraper::start_announcement_scraper_job(db.clone(), scraper_config).await;
    // index_daily_prices_sync::start_index_daily_prices_sync_job(db.clone()).await;

    // Configure CORS
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Build router
    let app = Router::new()
        .route("/", get(handlers::health::hello_indexmaker))
        .route("/indexes", get(handlers::index::get_index_list))
        .route("/add-token", post(handlers::token::add_token))
        .route("/add-tokens", post(handlers::token::add_tokens))
        .route("/create-index", post(handlers::index::create_index))
        .route("/get-index-config/{index_id}", get(handlers::index::get_index_config))
        .route("/save-blockchain-event", post(handlers::blockchain_event::save_blockchain_event))
        .route("/get-index-maker-info", get(handlers::index_maker::get_index_maker_info))
        .route("/get-deposit-transaction-data/{index_id}/{address}", get(handlers::deposit::get_deposit_transaction_data))
        .route("/fetch-coin-historical-data/{coin_id}", get(handlers::historical::fetch_coin_historical_data))
        .route("/indexes/{index_id}/transactions", get(handlers::transaction::get_index_transactions))
        .route("/download-daily-price-data/{index_id}", get(handlers::historical::download_daily_price_data))
        .route("/subscribe", post(handlers::subscription::subscribe))
        .route("/coingecko-categories", get(handlers::category::get_coingecko_categories))
        .route("/fetch-index-historical-data/{index_id}", get(handlers::historical::fetch_index_historical_data))
        .route("/indexes/{index_id}/price-at-date", get(handlers::index::get_index_price_at_date))
        .route("/indexes/{index_id}/last-price", get(handlers::index::get_index_last_price))
        .layer(cors)
        .with_state(state);

    // Start server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3002")
        .await
        .unwrap();

    tracing::info!("Server listening on {}", listener.local_addr().unwrap());

    axum::serve(listener, app).await.unwrap();
}