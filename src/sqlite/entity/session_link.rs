use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "session_links")]
pub(crate) struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) session_id: i64,
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) reference_id: i64,
    pub(crate) role: String,
    pub(crate) linked_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub(crate) enum Relation {
    #[sea_orm(
        belongs_to = "super::session::Entity",
        from = "Column::SessionId",
        to = "super::session::Column::Id",
        on_delete = "Cascade"
    )]
    Session,
    #[sea_orm(
        belongs_to = "super::work_item_reference::Entity",
        from = "Column::ReferenceId",
        to = "super::work_item_reference::Column::Id",
        on_delete = "Cascade"
    )]
    WorkItemReference,
}

impl Related<super::session::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Session.def()
    }
}

impl Related<super::work_item_reference::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::WorkItemReference.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
