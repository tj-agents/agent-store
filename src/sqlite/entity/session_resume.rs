use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "session_resumes")]
// This row contains only foreign keys; retain the schema's column names.
#[allow(clippy::struct_field_names)]
pub(crate) struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) session_id: i64,
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) source_launch_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) destination_launch_id: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub(crate) enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
