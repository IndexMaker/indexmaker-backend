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
                    .drop_column(IndexMetadata::TokenIds)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Recreate column (data will be lost)
        manager
            .alter_table(
                Table::alter()
                    .table(IndexMetadata::Table)
                    .add_column(
                        ColumnDef::new(IndexMetadata::TokenIds)
                            .array(ColumnType::Integer)
                            .not_null()
                            .default(Expr::cust("'{}'::integer[]")) // Empty array default
                    )
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum IndexMetadata {
    Table,
    TokenIds,
}