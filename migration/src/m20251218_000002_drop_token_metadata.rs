use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(TokenMetadata::Table).to_owned())
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Recreate table structure (data will be lost)
        manager
            .create_table(
                Table::create()
                    .table(TokenMetadata::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(TokenMetadata::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(TokenMetadata::Symbol).string().not_null())
                    .col(ColumnDef::new(TokenMetadata::LogoAddress).string())
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum TokenMetadata {
    Table,
    Id,
    Symbol,
    LogoAddress,
}