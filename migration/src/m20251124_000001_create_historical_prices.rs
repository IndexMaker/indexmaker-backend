use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(HistoricalPrices::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(HistoricalPrices::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(HistoricalPrices::CoinId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(HistoricalPrices::Symbol)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(HistoricalPrices::Timestamp)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(HistoricalPrices::Price)
                            .double()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Add index for efficient lookups
        manager
            .create_index(
                Index::create()
                    .name("idx_historical_prices_coin_timestamp")
                    .table(HistoricalPrices::Table)
                    .col(HistoricalPrices::CoinId)
                    .col(HistoricalPrices::Timestamp)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(HistoricalPrices::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum HistoricalPrices {
    Table,
    Id,
    CoinId,
    Symbol,
    Timestamp,
    Price,
}
