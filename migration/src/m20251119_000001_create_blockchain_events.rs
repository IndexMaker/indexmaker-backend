use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(BlockchainEvents::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(BlockchainEvents::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(BlockchainEvents::TxHash)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(BlockchainEvents::BlockNumber)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(BlockchainEvents::LogIndex)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(BlockchainEvents::EventType)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(BlockchainEvents::ContractAddress)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(BlockchainEvents::Network)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(BlockchainEvents::UserAddress).string())
                    .col(ColumnDef::new(BlockchainEvents::Amount).decimal())
                    .col(
                        ColumnDef::new(BlockchainEvents::Quantity)
                            .decimal()
                            .default("0"),
                    )
                    .col(
                        ColumnDef::new(BlockchainEvents::Timestamp)
                            .timestamp_with_time_zone(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(BlockchainEvents::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum BlockchainEvents {
    Table,
    Id,
    TxHash,
    BlockNumber,
    LogIndex,
    EventType,
    ContractAddress,
    Network,
    UserAddress,
    Amount,
    Quantity,
    Timestamp,
}