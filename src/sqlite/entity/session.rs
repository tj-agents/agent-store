use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "sessions")]
pub(crate) struct Model {
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    pub(crate) host: String,
    pub(crate) native_id: String,
    pub(crate) cwd: String,
    pub(crate) worktree_path: Option<String>,
    pub(crate) terminal_kind: Option<String>,
    pub(crate) terminal_id: Option<String>,
    pub(crate) wrapper_pid: Option<i64>,
    pub(crate) wrapper_started_at: Option<ChronoDateTimeUtc>,
    pub(crate) started_at: ChronoDateTimeUtc,
    pub(crate) closing_requested_at: Option<ChronoDateTimeUtc>,
    pub(crate) ended_at: Option<ChronoDateTimeUtc>,
    pub(crate) end_reason: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) parent: Option<String>,
    pub(crate) launch_id: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub(crate) enum Relation {
    #[sea_orm(has_many = "super::session_link::Entity")]
    SessionLink,
}

impl Related<super::session_link::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::SessionLink.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
