use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Rebalances::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Rebalances::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Rebalances::IndexId)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Rebalances::Coins)
                            .json_binary()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Rebalances::PortfolioValue)
                            .decimal()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Rebalances::TotalWeight)
                            .decimal()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Rebalances::Timestamp)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Rebalances::RebalanceType)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Rebalances::Deployed)
                            .boolean()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(Rebalances::DeployedAt)
                            .timestamp(),
                    )
                    .col(
                        ColumnDef::new(Rebalances::TxHash)
                            .string(),
                    )
                    .col(
                        ColumnDef::new(Rebalances::CreatedAt)
                            .timestamp()
                            .default(SimpleExpr::Keyword(Keyword::CurrentTimestamp)),
                    )
                    .to_owned(),
            )
            .await?;

        // Foreign key to index_metadata
        manager
            .create_foreign_key(
                ForeignKey::create()
                    .name("fk_rebalances_index_id")
                    .from(Rebalances::Table, Rebalances::IndexId)
                    .to(IndexMetadata::Table, IndexMetadata::IndexId)
                    .on_delete(ForeignKeyAction::Cascade)
                    .to_owned(),
            )
            .await?;

        // Unique constraint on index_id + timestamp
        manager
            .create_index(
                Index::create()
                    .name("idx_rebalances_unique")
                    .table(Rebalances::Table)
                    .col(Rebalances::IndexId)
                    .col(Rebalances::Timestamp)
                    .unique()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Rebalances::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Rebalances {
    Table,
    Id,
    IndexId,
    Coins,
    PortfolioValue,
    TotalWeight,
    Timestamp,
    RebalanceType,
    Deployed,
    DeployedAt,
    TxHash,
    CreatedAt,
}

#[derive(DeriveIden)]
enum IndexMetadata {
    Table,
    IndexId,
}
