use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "launches")]
pub(crate) struct Model {
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    pub(crate) launch_id: String,
    pub(crate) host: String,
    pub(crate) title: String,
    pub(crate) cwd: String,
    pub(crate) worktree_path: Option<String>,
    pub(crate) terminal_kind: Option<String>,
    pub(crate) terminal_id: Option<String>,
    pub(crate) wrapper_pid: i64,
    pub(crate) wrapper_started_at: ChronoDateTimeUtc,
    pub(crate) reference_id: Option<i64>,
    pub(crate) parent: Option<String>,
    pub(crate) created_at: ChronoDateTimeUtc,
    pub(crate) session_id: Option<i64>,
    pub(crate) closing_requested_at: Option<ChronoDateTimeUtc>,
    pub(crate) ended_at: Option<ChronoDateTimeUtc>,
    pub(crate) end_reason: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub(crate) enum Relation {
    #[sea_orm(
        belongs_to = "super::session::Entity",
        from = "Column::SessionId",
        to = "super::session::Column::Id",
        on_delete = "SetNull"
    )]
    Session,
    #[sea_orm(
        belongs_to = "super::work_item_reference::Entity",
        from = "Column::ReferenceId",
        to = "super::work_item_reference::Column::Id",
        on_delete = "SetNull"
    )]
    WorkItemReference,
}

impl ActiveModelBehavior for ActiveModel {}
