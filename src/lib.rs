// src/lib.rs

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
}