use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Coins::Table)
                    .add_column(
                        ColumnDef::new(Coins::LogoAddress)
                            .string()
                            .null()
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Coins::Table)
                    .drop_column(Coins::LogoAddress)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Coins {
    Table,
    LogoAddress,
}