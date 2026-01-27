//! Story 2-3: Add admin_address to itps table
//!
//! This migration adds the admin_address column to track which wallet address
//! created the ITP. This enables issuer portfolio/management views (AC #6).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add admin_address column (the wallet that created the ITP)
        manager
            .alter_table(
                Table::alter()
                    .table(Itps::Table)
                    .add_column(
                        ColumnDef::new(Itps::AdminAddress)
                            .string_len(42)
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Create index for querying ITPs by admin (issuer portfolio view)
        manager
            .create_index(
                Index::create()
                    .name("idx_itps_admin_address")
                    .table(Itps::Table)
                    .col(Itps::AdminAddress)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop index first
        manager
            .drop_index(
                Index::drop()
                    .name("idx_itps_admin_address")
                    .table(Itps::Table)
                    .to_owned(),
            )
            .await?;

        // Drop column
        manager
            .alter_table(
                Table::alter()
                    .table(Itps::Table)
                    .drop_column(Itps::AdminAddress)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(Iden)]
enum Itps {
    Table,
    AdminAddress,
}
