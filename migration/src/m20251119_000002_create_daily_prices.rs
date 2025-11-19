use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(DailyPrices::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(DailyPrices::IndexId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(DailyPrices::Date)
                            .date()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(DailyPrices::Price)
                            .decimal() // numeric in PostgreSQL
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(DailyPrices::Quantities)
                            .json_binary() // jsonb
                            .null(),
                    )
                    .col(
                        ColumnDef::new(DailyPrices::CreatedAt)
                            .timestamp()
                            .default(SimpleExpr::Keyword(Keyword::CurrentTimestamp)),
                    )
                    .col(
                        ColumnDef::new(DailyPrices::UpdatedAt)
                            .timestamp()
                            .default(SimpleExpr::Keyword(Keyword::CurrentTimestamp)),
                    )
                    .primary_key(
                        Index::create()
                            .col(DailyPrices::IndexId)
                            .col(DailyPrices::Date)
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(DailyPrices::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum DailyPrices {
    Table,
    IndexId,
    Date,
    Price,
    Quantities,
    CreatedAt,
    UpdatedAt,
}
