use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create index_constituents table
        manager
            .create_table(
                Table::create()
                    .table(IndexConstituents::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(IndexConstituents::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(IndexConstituents::IndexId)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(IndexConstituents::CoinId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(IndexConstituents::TokenSymbol)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(IndexConstituents::TokenName)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(IndexConstituents::Exchange)
                            .string()
                            .not_null()
                            .default("binance"),
                    )
                    .col(
                        ColumnDef::new(IndexConstituents::TradingPair)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(IndexConstituents::Position)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(IndexConstituents::AddedAt)
                            .timestamp()
                            .default(SimpleExpr::Keyword(Keyword::CurrentTimestamp)),
                    )
                    .col(
                        ColumnDef::new(IndexConstituents::RemovedAt)
                            .timestamp(),
                    )
                    .to_owned(),
            )
            .await?;

        // Foreign key to index_metadata
        manager
            .create_foreign_key(
                ForeignKey::create()
                    .name("fk_index_constituents_index_id")
                    .from(IndexConstituents::Table, IndexConstituents::IndexId)
                    .to(IndexMetadata::Table, IndexMetadata::IndexId)
                    .on_delete(ForeignKeyAction::Cascade)
                    .to_owned(),
            )
            .await?;

        // Unique constraint on index_id + coin_id
        manager
            .create_index(
                Index::create()
                    .name("idx_index_constituents_unique")
                    .table(IndexConstituents::Table)
                    .col(IndexConstituents::IndexId)
                    .col(IndexConstituents::CoinId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Index for efficient queries
        manager
            .create_index(
                Index::create()
                    .name("idx_index_constituents_lookup")
                    .table(IndexConstituents::Table)
                    .col(IndexConstituents::IndexId)
                    .col(IndexConstituents::RemovedAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop index_constituents table
        manager
            .drop_table(Table::drop().table(IndexConstituents::Table).to_owned())
            .await?;

        // Rename category back to coingecko_category
        manager
            .alter_table(
                Table::alter()
                    .table(IndexMetadata::Table)
                    .rename_column(IndexMetadata::Category, IndexMetadata::CoingeckoCategory)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum IndexConstituents {
    Table,
    Id,
    IndexId,
    CoinId,
    TokenSymbol,
    TokenName,
    Exchange,
    TradingPair,
    Position,
    AddedAt,
    RemovedAt,
}

#[derive(DeriveIden)]
enum IndexMetadata {
    Table,
    IndexId,
    CoingeckoCategory,
    Category,
}