use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create binance_listings view
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE VIEW binance_listings AS
                SELECT 
                    id,
                    CONCAT(UPPER(symbol), UPPER(trading_pair)) as pair,
                    CASE 
                        WHEN listing_date IS NOT NULL AND delisting_date IS NULL THEN 'listing'
                        WHEN delisting_date IS NOT NULL THEN 'delisting'
                        ELSE 'listing'
                    END as action,
                    CAST(
                        EXTRACT(EPOCH FROM COALESCE(listing_date, delisting_date, created_at)) * 1000 
                        AS BIGINT
                    ) as timestamp,
                    created_at
                FROM crypto_listings
                WHERE exchange = 'binance'
                ORDER BY COALESCE(listing_date, delisting_date, created_at) DESC;
                "#,
            )
            .await?;

        // Create bitget_listings view
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE VIEW bitget_listings AS
                SELECT 
                    id,
                    CONCAT(UPPER(symbol), UPPER(trading_pair)) as symbol,
                    UPPER(symbol) as base_asset,
                    UPPER(trading_pair) as quote_asset,
                    CASE 
                        WHEN LOWER(trading_pair) = 'usdt' THEN 'umcbl'
                        WHEN LOWER(trading_pair) = 'usdc' THEN 'cmcbl'
                        ELSE 'umcbl'
                    END as product_type,
                    CASE WHEN status = 'active' THEN true ELSE false END as status,
                    created_at,
                    updated_at
                FROM crypto_listings
                WHERE exchange = 'bitget'
                ORDER BY created_at DESC;
                "#,
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP VIEW IF EXISTS binance_listings")
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP VIEW IF EXISTS bitget_listings")
            .await?;

        Ok(())
    }
}