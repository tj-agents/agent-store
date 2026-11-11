use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{big_integer, string, string_null, timestamp_with_time_zone};

#[derive(DeriveMigrationName)]
pub(crate) struct Migration;

#[derive(DeriveIden)]
enum WorktreeOwnerships {
    Table,
    Path,
    SessionId,
    LaunchId,
    ClaimedAt,
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
