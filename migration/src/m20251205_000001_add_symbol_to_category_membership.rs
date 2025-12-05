use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(CategoryMembership::Table)
                    .add_column(ColumnDef::new(CategoryMembership::Symbol).string())
                    .to_owned(),
            )
            .await?;

        // Create index for fast symbol lookups
        manager
            .create_index(
                Index::create()
                    .name("idx_category_membership_symbol")
                    .table(CategoryMembership::Table)
                    .col(CategoryMembership::Symbol)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(CategoryMembership::Table)
                    .drop_column(CategoryMembership::Symbol)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum CategoryMembership {
    Table,
    Symbol,
}