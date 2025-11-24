pub use sea_orm_migration::prelude::*;

mod m20251117_000001_tokens_index;
mod m20251118_000001_add_index_fields;
mod m20251119_000001_create_blockchain_events;
mod m20251119_000002_create_daily_prices;
mod m20251124_000001_create_historical_prices;
mod m20251124_000002_create_subscriptions;

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
        ]
    }
}
