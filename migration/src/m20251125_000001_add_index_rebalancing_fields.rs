use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(IndexMetadata::Table)
                    .add_column(ColumnDef::new(IndexMetadata::InitialDate).date())
                    .add_column(ColumnDef::new(IndexMetadata::InitialPrice).decimal())
                    .add_column(ColumnDef::new(IndexMetadata::CoingeckoCategory).string())
                    .add_column(
                        ColumnDef::new(IndexMetadata::ExchangesAllowed)
                            .json_binary()
                    )
                    .add_column(
                        ColumnDef::new(IndexMetadata::ExchangeTradingFees)
                            .decimal()
                    )
                    .add_column(
                        ColumnDef::new(IndexMetadata::ExchangeAvgSpread)
                            .decimal()
                    )
                    .add_column(
                        ColumnDef::new(IndexMetadata::RebalancePeriod)
                            .integer()
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(IndexMetadata::Table)
                    .drop_column(IndexMetadata::InitialDate)
                    .drop_column(IndexMetadata::InitialPrice)
                    .drop_column(IndexMetadata::CoingeckoCategory)
                    .drop_column(IndexMetadata::ExchangesAllowed)
                    .drop_column(IndexMetadata::ExchangeTradingFees)
                    .drop_column(IndexMetadata::ExchangeAvgSpread)
                    .drop_column(IndexMetadata::RebalancePeriod)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum IndexMetadata {
    Table,
    InitialDate,
    InitialPrice,
    CoingeckoCategory,
    ExchangesAllowed,
    ExchangeTradingFees,
    ExchangeAvgSpread,
    RebalancePeriod,
}
