use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create token_metadata table
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
            .await?;

        // Create index_metadata table
        manager
            .create_table(
                Table::create()
                    .table(IndexMetadata::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(IndexMetadata::IndexId)
                            .integer()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(IndexMetadata::Category).string())
                    .col(ColumnDef::new(IndexMetadata::AssetClass).string())
                    .col(
                        ColumnDef::new(IndexMetadata::TokenIds)
                            .array(ColumnType::Integer)
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(IndexMetadata::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(TokenMetadata::Table).to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum TokenMetadata {
    Table,
    Id,
    Symbol,
    LogoAddress,
}

#[derive(DeriveIden)]
enum IndexMetadata {
    Table,
    IndexId,
    Category,
    AssetClass,
    TokenIds,
}
