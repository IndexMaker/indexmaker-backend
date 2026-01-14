use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(KeeperClaimableData::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(KeeperClaimableData::KeeperAddress)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(KeeperClaimableData::RecordedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(KeeperClaimableData::AcquisitionValue1)
                            .decimal()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(KeeperClaimableData::AcquisitionValue2)
                            .decimal()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(KeeperClaimableData::DisposalValue1)
                            .decimal()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(KeeperClaimableData::DisposalValue2)
                            .decimal()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(KeeperClaimableData::RawResponse)
                            .json_binary()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(KeeperClaimableData::CreatedAt)
                            .timestamp()
                            .default(SimpleExpr::Keyword(Keyword::CurrentTimestamp)),
                    )
                    .primary_key(
                        Index::create()
                            .col(KeeperClaimableData::KeeperAddress)
                            .col(KeeperClaimableData::RecordedAt),
                    )
                    .to_owned(),
            )
            .await?;

        // Add index for time-range queries
        manager
            .create_index(
                Index::create()
                    .name("idx_keeper_claimable_data_recorded_at")
                    .table(KeeperClaimableData::Table)
                    .col(KeeperClaimableData::RecordedAt)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(KeeperClaimableData::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum KeeperClaimableData {
    Table,
    KeeperAddress,
    RecordedAt,
    AcquisitionValue1,
    AcquisitionValue2,
    DisposalValue1,
    DisposalValue2,
    RawResponse,
    CreatedAt,
}
