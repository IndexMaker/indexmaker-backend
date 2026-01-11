pub use sea_orm_migration::prelude::*;

mod m20251117_000001_tokens_index;
mod m20251118_000001_add_index_fields;
mod m20251119_000001_create_blockchain_events;
mod m20251119_000002_create_daily_prices;
mod m20251124_000001_create_historical_prices;
mod m20251124_000002_create_subscriptions;
mod m20251125_000001_add_index_rebalancing_fields;
mod m20251125_000002_create_coingecko_categories;
mod m20251127_000001_create_crypto_listings;
mod m20251127_000002_create_announcements;
mod m20251127_000003_create_rebalances;
mod m20251127_000004_create_category_membership;
mod m20251201_000001_create_listing_views;
mod m20251203_000001_create_index_constituents;
mod m20251204_000001_index_add_deployment_data;
mod m20251205_000001_add_symbol_to_category_membership;
mod m20251208_000001_create_market_cap_rankings;
mod m20251209_000001_create_coins_table;
mod m20251210_000001_add_active_column_to_coins;
mod m20251210_000002_create_coins_historical_prices;
mod m20251216_000001_add_coins_historical_prices_indexes;
mod m20251217_000001_drop_old_historical_prices;
mod m20251217_000002_add_weight_strategy;
mod m20251218_000001_add_blacklisted_categories;
mod m20251218_000002_drop_token_metadata;
mod m20251218_000003_add_logo_address_to_coins;
mod m20251218_000004_drop_token_ids_from_index_metadata;
mod m20260111_000001_add_constituent_strategy;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20251117_000001_tokens_index::Migration),
            Box::new(m20251118_000001_add_index_fields::Migration),
            Box::new(m20251119_000001_create_blockchain_events::Migration),
            Box::new(m20251119_000002_create_daily_prices::Migration),
            Box::new(m20251124_000001_create_historical_prices::Migration),
            Box::new(m20251124_000002_create_subscriptions::Migration),
            Box::new(m20251125_000001_add_index_rebalancing_fields::Migration),
            Box::new(m20251125_000002_create_coingecko_categories::Migration),
            Box::new(m20251127_000001_create_crypto_listings::Migration),
            Box::new(m20251127_000002_create_announcements::Migration),
            Box::new(m20251127_000003_create_rebalances::Migration),
            Box::new(m20251127_000004_create_category_membership::Migration),
            Box::new(m20251201_000001_create_listing_views::Migration),
            Box::new(m20251203_000001_create_index_constituents::Migration),
            Box::new(m20251204_000001_index_add_deployment_data::Migration),
            Box::new(m20251205_000001_add_symbol_to_category_membership::Migration),
            Box::new(m20251208_000001_create_market_cap_rankings::Migration),
            Box::new(m20251209_000001_create_coins_table::Migration),
            Box::new(m20251210_000001_add_active_column_to_coins::Migration),
            Box::new(m20251210_000002_create_coins_historical_prices::Migration),
            Box::new(m20251216_000001_add_coins_historical_prices_indexes::Migration),
            Box::new(m20251217_000001_drop_old_historical_prices::Migration),
            Box::new(m20251217_000002_add_weight_strategy::Migration),
            Box::new(m20251218_000001_add_blacklisted_categories::Migration),
            Box::new(m20251218_000002_drop_token_metadata::Migration),
            Box::new(m20251218_000003_add_logo_address_to_coins::Migration),
            Box::new(m20251218_000004_drop_token_ids_from_index_metadata::Migration),
            Box::new(m20260111_000001_add_constituent_strategy::Migration),
        ]
    }
}
