use asset_registry::AssetRegistry;
use axum::{routing::{get, post}, Router};
use sea_orm::{Database, DatabaseConnection};
use sea_orm_migration::MigratorTrait;
use std::env;
use std::path::Path;
use std::sync::Arc;
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
    keeper_chart_sync,
    itp_price_snapshot_sync,
    itp_price_downsampler_job,
    bitget_historical_prices_sync,
    itp_chain_discovery_sync,
};
use services::coingecko::CoinGeckoService;
use services::itp_listing::ItpListingService;
use services::realtime_prices::RealTimePriceService;
use services::live_orderbook_cache::LiveOrderbookCache;
use services::bitget_ws_feeder::BitgetWsFeeder;
use handlers::operations_ws::OperationBroadcaster;

use crate::{jobs::{all_coingecko_coins_sync, coins_historical_prices_sync, coins_logo_sync}, scrapers::ScraperConfig, services::exchange_api::ExchangeApiService};

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub coingecko: CoinGeckoService,
    pub exchange_api: ExchangeApiService,
    pub itp_listing: ItpListingService,
    pub realtime_prices: RealTimePriceService,
    pub live_orderbook_cache: std::sync::Arc<LiveOrderbookCache>,
    /// Story 1-2: Shared asset registry for consistent ID mappings across all services
    pub asset_registry: Arc<AssetRegistry>,
    /// Story 3-2: Operation status broadcaster for WebSocket clients
    pub operation_broadcaster: Arc<OperationBroadcaster>,
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

    // Story 1-2: Load asset registry from vendor/assets.json
    // Priority: 1) ASSET_REGISTRY_PATH env var, 2) default paths
    let registry_path = env::var("ASSET_REGISTRY_PATH")
        .ok()
        .or_else(|| {
            let possible_paths = [
                "../vendor/assets.json",
                "vendor/assets.json",
            ];
            for path in &possible_paths {
                if Path::new(path).exists() {
                    return Some(path.to_string());
                }
            }
            None
        })
        .expect("Asset registry not found: set ASSET_REGISTRY_PATH or ensure vendor/assets.json exists");

    tracing::info!("Loading asset registry from: {}", registry_path);
    let asset_registry = AssetRegistry::load(Path::new(&registry_path))
        .expect("Failed to load asset registry - startup cannot continue");

    tracing::info!(
        event = "registry_loaded",
        asset_count = asset_registry.len(),
        service = "indexmaker-backend",
        "Asset registry loaded successfully"
    );

    let asset_registry = Arc::new(asset_registry);

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

    // Initialize ITP Listing service
    let itp_listing = ItpListingService::new();

    // Initialize Real-Time Price service (5 second polling from Binance/Bitget)
    let realtime_prices = RealTimePriceService::new(5);
    realtime_prices.start_polling();

    // Initialize Live Orderbook Cache and Bitget WebSocket Feeder
    let live_orderbook_cache = std::sync::Arc::new(LiveOrderbookCache::new());

    // Spawn task to start Bitget WebSocket feeds
    // Story 1-2: Use already-loaded registry instead of loading again
    let cache_clone = live_orderbook_cache.clone();
    let registry_clone = asset_registry.clone();
    tokio::spawn(async move {
        // Extract Bitget symbols from the shared registry
        let symbols: Vec<String> = registry_clone.all()
            .iter()
            .map(|a| a.bitget.clone())
            .collect();

        tracing::info!("Starting Bitget WebSocket feeder for {} symbols from registry", symbols.len());
        let mut feeder = BitgetWsFeeder::new(cache_clone);
        feeder.start(symbols).await;
    });

    // Story 3-2: Initialize operation broadcaster for WebSocket clients
    let operation_broadcaster = Arc::new(OperationBroadcaster::new());

    let state = AppState {
        db: db.clone(),
        coingecko: coingecko.clone(),
        exchange_api: exchange_api.clone(),
        itp_listing,
        realtime_prices,
        live_orderbook_cache,
        asset_registry: asset_registry.clone(),
        operation_broadcaster,
    };

    // Start background jobs
    // Job to fetch all coins in coingecko (only 1 api call per day)
    all_coingecko_coins_sync::start_all_coingecko_coins_sync_job(db.clone(), coingecko.clone()).await;

    // To find price of each token daily (1 api per coin at init - then for top 1000)
    coins_historical_prices_sync::start_coins_historical_prices_sync_job(db.clone(), coingecko.clone()).await;

    // Fetch logos for all coins from CoinGecko (persisted, only fetches missing logos)
    coins_logo_sync::start_coins_logo_sync_job(db.clone(), coingecko.clone()).await;

    // Fetch all categories (~750) in coingecko (1 api call)
    category_sync::start_category_sync_job(db.clone(), coingecko.clone()).await;

    // Finds coins related to each category - useful for blacklistings
    category_membership_sync::start_category_membership_sync_job(db.clone(), coingecko.clone()).await;

    // Rebalancer job, runs daily and check for rebalance period OR special (delisting) rebalancing
    rebalance_sync::start_rebalance_sync_job(db.clone(), coingecko.clone(), exchange_api.clone()).await;

    // Scraper service for Binance/Bitget
    announcement_scraper::start_announcement_scraper_job(db.clone(), scraper_config).await;

    // Computes price of each index (based on last rebalance quantities + coins daily prices)
    index_daily_prices_sync::start_index_daily_prices_sync_job(db.clone(), coingecko.clone()).await;

    // Keeper chart sync - polls Orbit VAULT for claimable data (Story 3.5)
    keeper_chart_sync::start_keeper_chart_sync_job(db.clone()).await;

    // ITP price snapshot - polls Castle for ITP prices every 5 minutes (Story 6.8)
    itp_price_snapshot_sync::start_itp_price_snapshot_job(db.clone()).await;

    // ITP price downsampler - aggregates old price data daily (Story 6.8)
    itp_price_downsampler_job::start_itp_price_downsampler_job(db.clone()).await;

    // Bitget historical prices - fetches historical prices from Bitget for all listed assets
    bitget_historical_prices_sync::start_bitget_historical_prices_sync_job(db.clone()).await;

    // ITP chain discovery - scans ItpCreated events on Arbitrum to discover bridge-deployed ITPs
    itp_chain_discovery_sync::start_itp_chain_discovery_job(db.clone(), asset_registry.clone()).await;

    // Configure CORS
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Build router
    let app = Router::new()
        .route("/", get(handlers::health::hello_indexmaker))
        .route("/indexes", get(handlers::index::get_index_list))
        .route("/create-index", post(handlers::index::create_index))
        .route("/api/index/manual", post(handlers::index::create_manual_index))
        .route("/api/index/{index_id}/rebalance", post(handlers::index::add_manual_rebalance))
        .route("/remove-index", post(handlers::index::remove_index))
        .route("/current-index-weight/{index_id}", get(handlers::index::get_current_index_weight))
        .route("/get-index-config/{index_id}", get(handlers::index::get_index_config))
        .route("/save-blockchain-event", post(handlers::blockchain_event::save_blockchain_event))
        .route("/get-index-maker-info", get(handlers::index_maker::get_index_maker_info))
        .route("/get-deposit-transaction-data/{index_id}/{address}", get(handlers::deposit::get_deposit_transaction_data))
        .route("/fetch-coin-historical-data/{coin_id}", get(handlers::historical::fetch_coin_historical_data))
        .route("/indexes/{index_id}/transactions", get(handlers::transaction::get_index_transactions))
        .route("/download-daily-price-data/{index_id}", get(handlers::historical::download_daily_price_data))
        .route("/subscribe", post(handlers::subscription::subscribe))
        .route("/coingecko-categories", get(handlers::category::get_coingecko_categories))
        .route("/api/categories/with-counts", get(handlers::category::get_categories_with_counts))
        .route("/fetch-index-historical-data/{index_id}", get(handlers::historical::fetch_index_historical_data))
        .route("/indexes/{index_id}/price-at-date", get(handlers::index::get_index_price_at_date))
        .route("/indexes/{index_id}/last-price", get(handlers::index::get_index_last_price))
        .route("/fetch-all-assets", get(handlers::asset::fetch_all_assets))
        .route("/fetch-vault-assets/{index_id}", get(handlers::asset::fetch_vault_assets))
        .route("/api/market-cap/history", get(handlers::market_cap::get_market_cap_history))
        .route("/api/market-cap/top-category", get(handlers::market_cap::get_top_category))
        .route("/api/market-cap/live-category", get(handlers::market_cap::get_live_category))
        .route("/api/exchange/tradeable-pairs", get(handlers::pairs::get_tradeable_pairs))
        .route("/api/exchange/all-tradeable-assets", get(handlers::pairs::get_all_tradeable_assets))
        .route("/api/coins/symbol-mapping", get(handlers::pairs::get_coin_symbol_mapping))
        // Keeper charts API (Story 3.5)
        .route("/api/keeper-charts/all", get(handlers::keeper_charts::get_all_keepers))
        .route("/api/keeper-charts/{keeper_address}/history", get(handlers::keeper_charts::get_keeper_history))
        .route("/api/keeper-charts/{keeper_address}/latest", get(handlers::keeper_charts::get_keeper_latest))
        // ITP creation API (Story 6.6)
        .route("/api/itp/create", post(handlers::itp::create_itp))
        // ITP status check API (real-time progress tracking)
        .route("/api/itp/status/{nonce}", get(handlers::itp::get_itp_status))
        // ITP listing API (Story 6.7)
        .route("/api/itp/list", get(handlers::itp_listing::get_itp_list))
        // ITP price history API (Story 6.8)
        .route("/api/itp/{id}/history", get(handlers::itp_history::get_itp_price_history))
        // ITP rebalance history API (Story 0-1 AC5)
        .route("/api/itp/{index_id}/rebalances", get(handlers::itp_rebalances::get_rebalance_history))
        // Virtual orderbook for index composition preview
        .route("/api/orderbook/virtual", post(handlers::orderbook::get_virtual_orderbook))
        // WebSocket for live orderbook streaming
        .route("/api/orderbook/ws", get(handlers::orderbook_ws::orderbook_websocket))
        .route("/api/orderbook/cache-stats", get(handlers::orderbook_ws::cache_stats))
        // Story 3.2: Operations status API and WebSocket
        .route("/api/operations", get(handlers::operations_ws::get_operations))
        .route("/api/operations/ws", get(handlers::operations_ws::operations_websocket))
        .route("/api/operations/update", post(handlers::operations_ws::update_operation))
        .layer(cors)
        .with_state(state);

    // Start server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3002")
        .await
        .unwrap();

    tracing::info!("Server listening on {}", listener.local_addr().unwrap());

    axum::serve(listener, app).await.unwrap();
}