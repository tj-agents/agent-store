use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    big_integer, big_integer_null, boolean, pk_auto, string, string_null, timestamp_with_time_zone,
    timestamp_with_time_zone_null,
};

#[derive(DeriveMigrationName)]
pub(crate) struct Migration;

#[derive(DeriveIden)]
enum WorkItemReferences {
    Table,
    Id,
    Provider,
    Scope,
    NativeId,
}

#[derive(DeriveIden)]
enum Sessions {
    Table,
    Id,
    Host,
    NativeId,
    Cwd,
    WorktreePath,
    TerminalKind,
    TerminalId,
    WrapperPid,
    WrapperStartedAt,
    StartedAt,
    ClosingRequestedAt,
    EndedAt,
    EndReason,
    Title,
    Parent,
    LaunchId,
}

#[derive(DeriveIden)]
enum SessionLinks {
    Table,
    SessionId,
    ReferenceId,
    Role,
    LinkedAt,
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
    Open,
}

#[derive(DeriveIden)]
enum WorktreeOwnerships {
    Table,
    Path,
    SessionId,
    LaunchId,
    ClaimedAt,
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
                    .table(WorkItemReferences::Table)
                    .col(pk_auto(WorkItemReferences::Id))
                    .col(string(WorkItemReferences::Provider))
                    .col(string(WorkItemReferences::Scope))
                    .col(string(WorkItemReferences::NativeId))
                    .index(
                        Index::create()
                            .unique()
                            .col(WorkItemReferences::Provider)
                            .col(WorkItemReferences::Scope)
                            .col(WorkItemReferences::NativeId),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Sessions::Table)
                    .col(pk_auto(Sessions::Id))
                    .col(string(Sessions::Host))
                    .col(string(Sessions::NativeId))
                    .col(string(Sessions::Cwd))
                    .col(string_null(Sessions::WorktreePath))
                    .col(string_null(Sessions::TerminalKind))
                    .col(string_null(Sessions::TerminalId))
                    .col(big_integer_null(Sessions::WrapperPid))
                    .col(timestamp_with_time_zone_null(Sessions::WrapperStartedAt))
                    .col(timestamp_with_time_zone(Sessions::StartedAt))
                    .col(timestamp_with_time_zone_null(Sessions::ClosingRequestedAt))
                    .col(timestamp_with_time_zone_null(Sessions::EndedAt))
                    .col(string_null(Sessions::EndReason))
                    .col(string_null(Sessions::Title))
                    .col(string_null(Sessions::Parent))
                    .col(string_null(Sessions::LaunchId))
                    .index(
                        Index::create()
                            .unique()
                            .col(Sessions::Host)
                            .col(Sessions::NativeId),
                    )
                    .to_owned(),
            )
            .await?;
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
                    .table(SessionLinks::Table)
                    .col(big_integer(SessionLinks::SessionId))
                    .col(big_integer(SessionLinks::ReferenceId))
                    .col(string(SessionLinks::Role))
                    .col(timestamp_with_time_zone(SessionLinks::LinkedAt))
                    .primary_key(
                        Index::create()
                            .col(SessionLinks::SessionId)
                            .col(SessionLinks::ReferenceId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(SessionLinks::Table, SessionLinks::SessionId)
                            .to(Sessions::Table, Sessions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(SessionLinks::Table, SessionLinks::ReferenceId)
                            .to(WorkItemReferences::Table, WorkItemReferences::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_session_links_reference")
                    .table(SessionLinks::Table)
                    .col(SessionLinks::ReferenceId)
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
                    .col(boolean(Launches::Open).default(false))
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
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(WorktreeOwnerships::Table)
                    .col(string(WorktreeOwnerships::Path).primary_key())
                    .col(big_integer(WorktreeOwnerships::SessionId))
                    .col(string_null(WorktreeOwnerships::LaunchId))
                    .col(timestamp_with_time_zone(WorktreeOwnerships::ClaimedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .from(WorktreeOwnerships::Table, WorktreeOwnerships::SessionId)
                            .to(Sessions::Table, Sessions::Id),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(WorktreeOwnerships::Table, WorktreeOwnerships::LaunchId)
                            .to(Launches::Table, Launches::LaunchId),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_worktree_ownerships_owner")
                    .table(WorktreeOwnerships::Table)
                    .col(WorktreeOwnerships::SessionId)
                    .col(WorktreeOwnerships::LaunchId)
                    .to_owned(),
            )
            .await
    }
}
