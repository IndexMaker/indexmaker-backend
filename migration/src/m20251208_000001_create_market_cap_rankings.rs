use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MarketCapRankings::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(MarketCapRankings::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(MarketCapRankings::Date)
                            .date()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MarketCapRankings::CoinId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MarketCapRankings::Symbol)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MarketCapRankings::MarketCap)
                            .decimal()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MarketCapRankings::Price)
                            .decimal()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MarketCapRankings::Rank)
                            .integer(),
                    )
                    .col(
                        ColumnDef::new(MarketCapRankings::CreatedAt)
                            .timestamp()
                            .default(SimpleExpr::Keyword(Keyword::CurrentTimestamp)),
                    )
                    .to_owned(),
            )
            .await?;

        // Unique constraint on date + coin_id
        manager
            .create_index(
                Index::create()
                    .name("idx_market_cap_rankings_unique")
                    .table(MarketCapRankings::Table)
                    .col(MarketCapRankings::Date)
                    .col(MarketCapRankings::CoinId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Index for fast lookups by date and rank
        manager
            .create_index(
                Index::create()
                    .name("idx_market_cap_date_rank")
                    .table(MarketCapRankings::Table)
                    .col(MarketCapRankings::Date)
                    .col(MarketCapRankings::Rank)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(MarketCapRankings::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum MarketCapRankings {
    Table,
    Id,
    Date,
    CoinId,
    Symbol,
    MarketCap,
    Price,
    Rank,
    CreatedAt,
}