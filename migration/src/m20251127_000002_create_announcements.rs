use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Announcements::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Announcements::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Announcements::Title)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Announcements::Source)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Announcements::AnnounceDate)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Announcements::Content)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Announcements::Parsed)
                            .boolean()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(Announcements::AnnouncementType)
                            .string(),
                    )
                    .col(
                        ColumnDef::new(Announcements::Url)
                            .string(),
                    )
                    .col(
                        ColumnDef::new(Announcements::CreatedAt)
                            .timestamp()
                            .default(SimpleExpr::Keyword(Keyword::CurrentTimestamp)),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Announcements::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Announcements {
    Table,
    Id,
    Title,
    Source,
    AnnounceDate,
    Content,
    Parsed,
    AnnouncementType,
    Url,
    CreatedAt,
}
