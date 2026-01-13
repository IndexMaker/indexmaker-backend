// src/lib.rs

use sea_orm::DatabaseConnection;
use services::{coingecko::CoinGeckoService, exchange_api::ExchangeApiService};

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub coingecko: CoinGeckoService,
    pub exchange_api: ExchangeApiService,
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
}

pub mod models;
pub mod handlers;