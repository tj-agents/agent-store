use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "worktree_ownerships")]
pub(crate) struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub path: String,
    pub session_id: i64,
    pub launch_id: Option<String>,
    pub claimed_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub(crate) enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
