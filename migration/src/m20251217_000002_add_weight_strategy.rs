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
                    .add_column(
                        ColumnDef::new(IndexMetadata::WeightStrategy)
                            .string()
                            .not_null()
                            .default("equal")
                    )
                    .add_column(
                        ColumnDef::new(IndexMetadata::WeightThreshold)
                            .decimal()
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
                    .table(IndexMetadata::Table)
                    .drop_column(IndexMetadata::WeightStrategy)
                    .drop_column(IndexMetadata::WeightThreshold)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum IndexMetadata {
    Table,
    WeightStrategy,
    WeightThreshold,
}