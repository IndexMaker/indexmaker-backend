use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create itp_price_history table
        manager
            .create_table(
                Table::create()
                    .table(ItpPriceHistory::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ItpPriceHistory::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ItpPriceHistory::ItpId)
                            .string_len(66)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ItpPriceHistory::Price)
                            .decimal_len(78, 18)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ItpPriceHistory::Volume)
                            .decimal_len(78, 18)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ItpPriceHistory::Timestamp)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ItpPriceHistory::Granularity)
                            .string_len(10)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ItpPriceHistory::CreatedAt)
                            .timestamp_with_time_zone()
                            .default(SimpleExpr::Keyword(Keyword::CurrentTimestamp)),
                    )
                    .to_owned(),
            )
            .await?;

        // Create composite index for fast lookups: (itp_id, timestamp DESC)
        manager
            .create_index(
                Index::create()
                    .name("idx_itp_price_history_itp_time")
                    .table(ItpPriceHistory::Table)
                    .col(ItpPriceHistory::ItpId)
                    .col((ItpPriceHistory::Timestamp, IndexOrder::Desc))
                    .to_owned(),
            )
            .await?;

        // Create index for cleanup/downsampling: (granularity, timestamp)
        manager
            .create_index(
                Index::create()
                    .name("idx_itp_price_history_granularity_time")
                    .table(ItpPriceHistory::Table)
                    .col(ItpPriceHistory::Granularity)
                    .col(ItpPriceHistory::Timestamp)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ItpPriceHistory::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum ItpPriceHistory {
    Table,
    Id,
    ItpId,
    Price,
    Volume,
    Timestamp,
    Granularity,
    CreatedAt,
}
