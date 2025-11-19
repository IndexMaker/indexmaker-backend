pub use sea_orm_migration::prelude::*;

mod m20251117_000001_tokens_index;
mod m20251118_000001_add_index_fields;
mod m20251119_000001_create_blockchain_events;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20251117_000001_tokens_index::Migration),
            Box::new(m20251118_000001_add_index_fields::Migration),
            Box::new(m20251119_000001_create_blockchain_events::Migration),
        ]
    }
}
