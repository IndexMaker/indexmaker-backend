use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Coins::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Coins::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Coins::CoinId)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(Coins::Symbol)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Coins::Name)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Coins::Platforms)
                            .json_binary()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Coins::ActivatedAt)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Coins::CreatedAt)
                            .timestamp()
                            .default(SimpleExpr::Keyword(Keyword::CurrentTimestamp)),
                    )
                    .col(
                        ColumnDef::new(Coins::UpdatedAt)
                            .timestamp()
                            .default(SimpleExpr::Keyword(Keyword::CurrentTimestamp)),
                    )
                    .to_owned(),
            )
            .await?;

        // Create index for fast symbol lookups
        manager
            .create_index(
                Index::create()
                    .name("idx_coins_symbol")
                    .table(Coins::Table)
                    .col(Coins::Symbol)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Coins::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Coins {
    Table,
    Id,
    CoinId,
    Symbol,
    Name,
    Platforms,
    ActivatedAt,
    CreatedAt,
    UpdatedAt,
}