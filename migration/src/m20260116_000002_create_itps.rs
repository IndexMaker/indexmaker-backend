use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create itps table
        manager
            .create_table(
                Table::create()
                    .table(Itps::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Itps::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Itps::OrbitAddress)
                            .string_len(42)
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(Itps::ArbitrumAddress)
                            .string_len(42)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Itps::IndexId)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Itps::Name)
                            .string_len(255)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Itps::Symbol)
                            .string_len(20)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Itps::InitialPrice)
                            .decimal_len(78, 0)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Itps::CurrentPrice)
                            .decimal_len(78, 0)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Itps::TotalSupply)
                            .decimal_len(78, 0)
                            .default("0"),
                    )
                    .col(
                        ColumnDef::new(Itps::State)
                            .small_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(Itps::CreatedAt)
                            .timestamp_with_time_zone()
                            .default(SimpleExpr::Keyword(Keyword::CurrentTimestamp)),
                    )
                    .col(
                        ColumnDef::new(Itps::UpdatedAt)
                            .timestamp_with_time_zone()
                            .default(SimpleExpr::Keyword(Keyword::CurrentTimestamp)),
                    )
                    .col(
                        ColumnDef::new(Itps::DeployTxHash)
                            .string_len(66)
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Create index on symbol for fast lookups
        manager
            .create_index(
                Index::create()
                    .name("idx_itps_symbol")
                    .table(Itps::Table)
                    .col(Itps::Symbol)
                    .to_owned(),
            )
            .await?;

        // Create index on state for filtering active ITPs
        manager
            .create_index(
                Index::create()
                    .name("idx_itps_state")
                    .table(Itps::Table)
                    .col(Itps::State)
                    .to_owned(),
            )
            .await?;

        // Create index on orbit_address for fast lookups
        manager
            .create_index(
                Index::create()
                    .name("idx_itps_orbit_address")
                    .table(Itps::Table)
                    .col(Itps::OrbitAddress)
                    .to_owned(),
            )
            .await?;

        // Create trigger function for updated_at (if not exists)
        let db = manager.get_connection();
        db.execute_unprepared(
            r#"
            CREATE OR REPLACE FUNCTION update_itps_updated_at()
            RETURNS TRIGGER AS $$
            BEGIN
                NEW.updated_at = NOW();
                RETURN NEW;
            END;
            $$ LANGUAGE plpgsql;
            "#,
        )
        .await?;

        // Create trigger on itps table
        db.execute_unprepared(
            r#"
            DROP TRIGGER IF EXISTS trigger_itps_updated_at ON itps;
            CREATE TRIGGER trigger_itps_updated_at
                BEFORE UPDATE ON itps
                FOR EACH ROW
                EXECUTE FUNCTION update_itps_updated_at();
            "#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Drop trigger and function
        db.execute_unprepared("DROP TRIGGER IF EXISTS trigger_itps_updated_at ON itps;")
            .await?;
        db.execute_unprepared("DROP FUNCTION IF EXISTS update_itps_updated_at();")
            .await?;

        manager
            .drop_table(Table::drop().table(Itps::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum Itps {
    Table,
    Id,
    OrbitAddress,
    ArbitrumAddress,
    IndexId,
    Name,
    Symbol,
    InitialPrice,
    CurrentPrice,
    TotalSupply,
    State,
    CreatedAt,
    UpdatedAt,
    DeployTxHash,
}
