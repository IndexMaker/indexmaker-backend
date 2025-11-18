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
                    .add_column(ColumnDef::new(IndexMetadata::Name).string().not_null().default(""))
                    .add_column(ColumnDef::new(IndexMetadata::Symbol).string().not_null().default(""))
                    .add_column(ColumnDef::new(IndexMetadata::Address).string().not_null().default(""))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(IndexMetadata::Table)
                    .drop_column(IndexMetadata::Name)
                    .drop_column(IndexMetadata::Symbol)
                    .drop_column(IndexMetadata::Address)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum IndexMetadata {
    Table,
    Name,
    Symbol,
    Address,
}
