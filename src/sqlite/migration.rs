mod m20261108_000001_create_link_store;

use sea_orm_migration::{MigrationTrait, MigratorTrait};

pub(crate) struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m20261108_000001_create_link_store::Migration)]
    }
}
