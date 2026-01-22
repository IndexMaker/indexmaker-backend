// src/lib.rs

use sea_orm::DatabaseConnection;
use std::sync::Arc;
use services::{
    coingecko::CoinGeckoService,
    exchange_api::ExchangeApiService,
    itp_listing::ItpListingService,
    realtime_prices::RealTimePriceService,
    live_orderbook_cache::LiveOrderbookCache,
};

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub coingecko: CoinGeckoService,
    pub exchange_api: ExchangeApiService,
    pub itp_listing: ItpListingService,
    pub realtime_prices: RealTimePriceService,
    pub live_orderbook_cache: Arc<LiveOrderbookCache>,
}

pub mod entities {
    pub mod prelude;
    pub mod announcements;
    pub mod blockchain_events;
    pub mod category_membership;
    pub mod coingecko_categories;
    pub mod crypto_listings;
    pub mod daily_prices;
    pub mod index_metadata;
    pub mod rebalances;
    pub mod subscriptions;
    pub mod index_constituents;
    pub mod market_cap_rankings;
    pub mod coins;
    pub mod coins_historical_prices;
    pub mod keeper_claimable_data;
    pub mod itp_price_history;
    pub mod itps;
}

pub mod services {
    pub mod coingecko;
    pub mod exchange_api;
    pub mod rebalancing;
    pub mod price_utils;
    pub mod market_cap;
    pub mod constituent_selector;
    pub mod weight_calculator;
    pub mod daily_prices;
    pub mod category_service;
    pub mod orbit_keeper;
    pub mod itp_creation;
    pub mod itp_listing;
    pub mod itp_price_snapshot;
    pub mod itp_price_downsampler;
    pub mod realtime_prices;
    pub mod bitget_websocket;
    pub mod orderbook_aggregator;
    pub mod live_orderbook_cache;
    pub mod bitget_ws_feeder;
}

pub mod models;
pub mod handlers;