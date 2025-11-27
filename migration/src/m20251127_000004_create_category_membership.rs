use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(CategoryMembership::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(CategoryMembership::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(CategoryMembership::CoinId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CategoryMembership::CategoryId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CategoryMembership::AddedDate)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CategoryMembership::RemovedDate)
                            .timestamp(),
                    )
                    .col(
                        ColumnDef::new(CategoryMembership::CreatedAt)
                            .timestamp()
                            .default(SimpleExpr::Keyword(Keyword::CurrentTimestamp)),
                    )
                    .col(
                        ColumnDef::new(CategoryMembership::UpdatedAt)
                            .timestamp()
                            .default(SimpleExpr::Keyword(Keyword::CurrentTimestamp)),
                    )
                    .to_owned(),
            )
            .await?;

        // Create index for fast lookups
        manager
            .create_index(
                Index::create()
                    .name("idx_category_membership_lookup")
                    .table(CategoryMembership::Table)
                    .col(CategoryMembership::CategoryId)
                    .col(CategoryMembership::CoinId)
                    .col(CategoryMembership::AddedDate)
                    .col(CategoryMembership::RemovedDate)
                    .to_owned(),
            )
            .await?;

        // Create unique index to prevent duplicate active memberships
        manager
            .create_index(
                Index::create()
                    .name("idx_category_membership_active")
                    .table(CategoryMembership::Table)
                    .col(CategoryMembership::CoinId)
                    .col(CategoryMembership::CategoryId)
                    .col(CategoryMembership::RemovedDate)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(CategoryMembership::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum CategoryMembership {
    Table,
    Id,
    CoinId,
    CategoryId,
    AddedDate,
    RemovedDate,
    CreatedAt,
    UpdatedAt,
}
