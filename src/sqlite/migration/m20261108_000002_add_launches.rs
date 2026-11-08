use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    big_integer, big_integer_null, pk_auto, string, string_null, timestamp_with_time_zone,
    timestamp_with_time_zone_null,
};

#[derive(DeriveMigrationName)]
pub(crate) struct Migration;

#[derive(DeriveIden)]
enum Sessions {
    Table,
    Id,
    Title,
    Parent,
    LaunchId,
}

#[derive(DeriveIden)]
enum WorkItemReferences {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Launches {
    Table,
    Id,
    LaunchId,
    Host,
    Title,
    Cwd,
    WorktreePath,
    TerminalKind,
    TerminalId,
    WrapperPid,
    WrapperStartedAt,
    ReferenceId,
    Parent,
    CreatedAt,
    SessionId,
    ClosingRequestedAt,
    EndedAt,
    EndReason,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }

    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite adds one column per ALTER TABLE.
        for column in [Sessions::Title, Sessions::Parent, Sessions::LaunchId] {
            manager
                .alter_table(
                    Table::alter()
                        .table(Sessions::Table)
                        .add_column(string_null(column))
                        .to_owned(),
                )
                .await?;
        }
        manager
            .create_index(
                Index::create()
                    .name("idx_sessions_title")
                    .table(Sessions::Table)
                    .col(Sessions::Title)
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Launches::Table)
                    .col(pk_auto(Launches::Id))
                    .col(string(Launches::LaunchId).unique_key())
                    .col(string(Launches::Host))
                    .col(string(Launches::Title))
                    .col(string(Launches::Cwd))
                    .col(string_null(Launches::WorktreePath))
                    .col(string_null(Launches::TerminalKind))
                    .col(string_null(Launches::TerminalId))
                    .col(big_integer(Launches::WrapperPid))
                    .col(timestamp_with_time_zone(Launches::WrapperStartedAt))
                    .col(big_integer_null(Launches::ReferenceId))
                    .col(string_null(Launches::Parent))
                    .col(timestamp_with_time_zone(Launches::CreatedAt))
                    .col(big_integer_null(Launches::SessionId))
                    .col(timestamp_with_time_zone_null(Launches::ClosingRequestedAt))
                    .col(timestamp_with_time_zone_null(Launches::EndedAt))
                    .col(string_null(Launches::EndReason))
                    .foreign_key(
                        ForeignKey::create()
                            .from(Launches::Table, Launches::ReferenceId)
                            .to(WorkItemReferences::Table, WorkItemReferences::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(Launches::Table, Launches::SessionId)
                            .to(Sessions::Table, Sessions::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_launches_reference")
                    .table(Launches::Table)
                    .col(Launches::ReferenceId)
                    .to_owned(),
            )
            .await
    }
}
