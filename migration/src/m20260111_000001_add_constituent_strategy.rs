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
                        ColumnDef::new(IndexMetadata::ConstituentSelectionStrategy)
                            .string()
                            .null()
                            .comment("Strategy for selecting constituents: 'fixed', 'top_market_cap', or 'category'")
                    )
                    .add_column(
                        ColumnDef::new(IndexMetadata::TopN)
                            .integer()
                            .null()
                            .comment("Number of top coins to select (only used with 'top_market_cap' strategy)")
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
                    .drop_column(IndexMetadata::ConstituentSelectionStrategy)
                    .drop_column(IndexMetadata::TopN)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum IndexMetadata {
    Table,
    ConstituentSelectionStrategy,
    TopN,
}
