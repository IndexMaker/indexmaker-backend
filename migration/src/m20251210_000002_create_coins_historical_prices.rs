use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(CoinsHistoricalPrices::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(CoinsHistoricalPrices::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(CoinsHistoricalPrices::CoinId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CoinsHistoricalPrices::Symbol)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CoinsHistoricalPrices::Date)
                            .date()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CoinsHistoricalPrices::Price)
                            .decimal()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CoinsHistoricalPrices::MarketCap)
                            .decimal()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(CoinsHistoricalPrices::Volume)
                            .decimal()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(CoinsHistoricalPrices::CreatedAt)
                            .timestamp()
                            .default(SimpleExpr::Keyword(Keyword::CurrentTimestamp)),
                    )
                    .to_owned(),
            )
            .await?;

        // Unique constraint: one price per coin per date
        manager
            .create_index(
                Index::create()
                    .name("idx_coins_historical_prices_unique")
                    .table(CoinsHistoricalPrices::Table)
                    .col(CoinsHistoricalPrices::CoinId)
                    .col(CoinsHistoricalPrices::Date)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Index for date-based market cap rankings
        manager
            .create_index(
                Index::create()
                    .name("idx_coins_historical_prices_date_mcap")
                    .table(CoinsHistoricalPrices::Table)
                    .col(CoinsHistoricalPrices::Date)
                    .col(CoinsHistoricalPrices::MarketCap)
                    .to_owned(),
            )
            .await?;

        // Index for symbol lookups
        manager
            .create_index(
                Index::create()
                    .name("idx_coins_historical_prices_symbol_date")
                    .table(CoinsHistoricalPrices::Table)
                    .col(CoinsHistoricalPrices::Symbol)
                    .col(CoinsHistoricalPrices::Date)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(CoinsHistoricalPrices::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum CoinsHistoricalPrices {
    Table,
    Id,
    CoinId,
    Symbol,
    Date,
    Price,
    MarketCap,
    Volume,
    CreatedAt,
}