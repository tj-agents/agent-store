use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "work_item_references")]
pub(crate) struct Model {
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    pub(crate) provider: String,
    pub(crate) scope: String,
    pub(crate) native_id: String,
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
