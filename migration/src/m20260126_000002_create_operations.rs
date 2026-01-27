//! Migration to create the operations table for tracking buy/sell/rebalance operations
//!
//! Story 3.2 - AC #10, NFR16: Operation state persistence

use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Operations::Table)
                    .if_not_exists()
                    .col(pk_auto(Operations::Id))
                    .col(string(Operations::UserAddress).not_null())
                    .col(string(Operations::OperationType).not_null())
                    .col(big_unsigned(Operations::Nonce).not_null())
                    .col(string(Operations::Status).not_null())
                    .col(string_null(Operations::ArbTxHash))
                    .col(string_null(Operations::OrbitTxHash))
                    .col(string_null(Operations::CompletionTxHash))
                    .col(string_null(Operations::Amount))
                    .col(string_null(Operations::ItpAmount))
                    .col(string_null(Operations::ItpAddress))
                    .col(string_null(Operations::ErrorCode))
                    .col(string_null(Operations::ErrorMessage))
                    .col(boolean(Operations::Retryable).default(false))
                    .col(timestamp(Operations::CreatedAt).default(Expr::current_timestamp()))
                    .col(timestamp(Operations::UpdatedAt).default(Expr::current_timestamp()))
                    .to_owned(),
            )
            .await?;

        // Index for querying by user address
        manager
            .create_index(
                Index::create()
                    .name("idx_operations_user_address")
                    .table(Operations::Table)
                    .col(Operations::UserAddress)
                    .to_owned(),
            )
            .await?;

        // Index for querying by user + nonce (unique per user)
        manager
            .create_index(
                Index::create()
                    .name("idx_operations_user_nonce")
                    .table(Operations::Table)
                    .col(Operations::UserAddress)
                    .col(Operations::Nonce)
                    .col(Operations::OperationType)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Index for querying by status (for pending operations)
        manager
            .create_index(
                Index::create()
                    .name("idx_operations_status")
                    .table(Operations::Table)
                    .col(Operations::Status)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Operations::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Operations {
    Table,
    Id,
    UserAddress,
    OperationType,
    Nonce,
    Status,
    ArbTxHash,
    OrbitTxHash,
    CompletionTxHash,
    Amount,
    ItpAmount,
    ItpAddress,
    ErrorCode,
    ErrorMessage,
    Retryable,
    CreatedAt,
    UpdatedAt,
}
