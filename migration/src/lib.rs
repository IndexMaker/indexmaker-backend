pub use sea_orm_migration::prelude::*;

mod m20251117_000001_tokens_index;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20251117_000001_tokens_index::Migration),
        ]
    }
}
