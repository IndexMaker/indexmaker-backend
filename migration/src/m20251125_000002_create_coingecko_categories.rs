use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(CoingeckoCategories::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(CoingeckoCategories::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(CoingeckoCategories::CategoryId)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(CoingeckoCategories::Name)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CoingeckoCategories::UpdatedAt)
                            .timestamp()
                            .default(SimpleExpr::Keyword(Keyword::CurrentTimestamp)),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(CoingeckoCategories::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum CoingeckoCategories {
    Table,
    Id,
    CategoryId,
    Name,
    UpdatedAt,
}
