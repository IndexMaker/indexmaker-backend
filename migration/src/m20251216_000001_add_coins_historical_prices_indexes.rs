use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Index 1: For fast coin_id + date lookups
        // Used by: get_or_fetch_coins_historical_price() and general price queries
        manager
            .create_index(
                Index::create()
                    .name("idx_coins_historical_prices_coin_date")
                    .table(CoinsHistoricalPrices::Table)
                    .col(CoinsHistoricalPrices::CoinId)
                    .col((CoinsHistoricalPrices::Date, IndexOrder::Desc))
                    .to_owned(),
            )
            .await?;

        // Index 2: For the batch DISTINCT ON query with market_cap sorting
        // Used by: get_all_coins_last_dates_batch() in sync job
        // We need raw SQL for NULLS LAST on market_cap
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE INDEX idx_coins_historical_prices_distinct_on 
                ON coins_historical_prices(coin_id, date DESC, market_cap DESC NULLS LAST)
                "#
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop indexes in reverse order
        manager
            .drop_index(
                Index::drop()
                    .name("idx_coins_historical_prices_distinct_on")
                    .table(CoinsHistoricalPrices::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx_coins_historical_prices_coin_date")
                    .table(CoinsHistoricalPrices::Table)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
#[allow(dead_code)]
enum CoinsHistoricalPrices {
    Table,
    CoinId,
    Date,
    MarketCap,
}