use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{big_integer, string};

#[derive(DeriveMigrationName)]
pub(crate) struct Migration;

#[derive(DeriveIden)]
enum SessionResumes {
    Table,
    SessionId,
    SourceLaunchId,
    DestinationLaunchId,
}

#[derive(DeriveIden)]
enum Sessions {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Launches {
    Table,
    LaunchId,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }

    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(SessionResumes::Table)
                    .col(big_integer(SessionResumes::SessionId))
                    .col(string(SessionResumes::SourceLaunchId))
                    .col(string(SessionResumes::DestinationLaunchId))
                    .primary_key(
                        Index::create()
                            .col(SessionResumes::SessionId)
                            .col(SessionResumes::SourceLaunchId)
                            .col(SessionResumes::DestinationLaunchId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(SessionResumes::Table, SessionResumes::SessionId)
                            .to(Sessions::Table, Sessions::Id),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(SessionResumes::Table, SessionResumes::SourceLaunchId)
                            .to(Launches::Table, Launches::LaunchId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(SessionResumes::Table, SessionResumes::DestinationLaunchId)
                            .to(Launches::Table, Launches::LaunchId),
                    )
                    .to_owned(),
            )
            .await
    }
}
