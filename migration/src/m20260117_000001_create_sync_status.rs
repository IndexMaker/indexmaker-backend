use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create sync_status table to track last successful sync times for each job
        manager
            .create_table(
                Table::create()
                    .table(SyncStatus::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SyncStatus::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(SyncStatus::JobName)
                            .string_len(100)
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(SyncStatus::LastSuccessAt)
                            .timestamp()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(SyncStatus::LastAttemptAt)
                            .timestamp()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(SyncStatus::LastError)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(SyncStatus::SuccessCount)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(SyncStatus::ErrorCount)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(SyncStatus::MinIntervalSecs)
                            .integer()
                            .not_null()
                            .default(3600), // Default 1 hour minimum between syncs
                    )
                    .to_owned(),
            )
            .await?;

        // Create index on job_name for fast lookups
        manager
            .create_index(
                Index::create()
                    .name("idx_sync_status_job_name")
                    .table(SyncStatus::Table)
                    .col(SyncStatus::JobName)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(SyncStatus::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum SyncStatus {
    Table,
    Id,
    JobName,
    LastSuccessAt,
    LastAttemptAt,
    LastError,
    SuccessCount,
    ErrorCount,
    MinIntervalSecs,
}
