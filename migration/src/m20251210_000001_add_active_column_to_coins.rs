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
                        ColumnDef::new(Coins::Active)
                            .boolean()
                            .not_null()
                            .default(true)
                    )
                    .to_owned(),
            )
            .await?;

        // Create index for fast active/inactive filtering
        manager
            .create_index(
                Index::create()
                    .name("idx_coins_active")
                    .table(Coins::Table)
                    .col(Coins::Active)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Coins::Table)
                    .drop_column(Coins::Active)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Coins {
    Table,
    Active,
}