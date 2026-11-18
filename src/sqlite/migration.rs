mod m20261112_000001_create_store;
mod m20261118_000001_record_resumes;

use sea_orm_migration::{MigrationTrait, MigratorTrait};

pub(crate) struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20261112_000001_create_store::Migration),
            Box::new(m20261118_000001_record_resumes::Migration),
        ]
    }
}
