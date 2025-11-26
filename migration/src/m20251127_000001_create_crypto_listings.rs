use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(CryptoListings::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(CryptoListings::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(CryptoListings::CoinId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CryptoListings::Symbol)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CryptoListings::TokenName)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CryptoListings::Exchange)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CryptoListings::TradingPair)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CryptoListings::ListingAnnouncementDate)
                            .timestamp(),
                    )
                    .col(
                        ColumnDef::new(CryptoListings::ListingDate)
                            .timestamp(),
                    )
                    .col(
                        ColumnDef::new(CryptoListings::DelistingAnnouncementDate)
                            .timestamp(),
                    )
                    .col(
                        ColumnDef::new(CryptoListings::DelistingDate)
                            .timestamp(),
                    )
                    .col(
                        ColumnDef::new(CryptoListings::Status)
                            .string()
                            .not_null()
                            .default("active"),
                    )
                    .col(
                        ColumnDef::new(CryptoListings::CreatedAt)
                            .timestamp()
                            .default(SimpleExpr::Keyword(Keyword::CurrentTimestamp)),
                    )
                    .col(
                        ColumnDef::new(CryptoListings::UpdatedAt)
                            .timestamp()
                            .default(SimpleExpr::Keyword(Keyword::CurrentTimestamp)),
                    )
                    .to_owned(),
            )
            .await?;

        // Create unique constraint
        manager
            .create_index(
                Index::create()
                    .name("idx_crypto_listings_unique")
                    .table(CryptoListings::Table)
                    .col(CryptoListings::CoinId)
                    .col(CryptoListings::Exchange)
                    .col(CryptoListings::TradingPair)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Create index for fast lookups
        manager
            .create_index(
                Index::create()
                    .name("idx_crypto_listings_lookup")
                    .table(CryptoListings::Table)
                    .col(CryptoListings::CoinId)
                    .col(CryptoListings::Exchange)
                    .col(CryptoListings::TradingPair)
                    .col(CryptoListings::ListingDate)
                    .col(CryptoListings::DelistingDate)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(CryptoListings::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum CryptoListings {
    Table,
    Id,
    CoinId,
    Symbol,
    TokenName,
    Exchange,
    TradingPair,
    ListingAnnouncementDate,
    ListingDate,
    DelistingAnnouncementDate,
    DelistingDate,
    Status,
    CreatedAt,
    UpdatedAt,
}
