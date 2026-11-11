mod m20261108_000001_create_link_store;
mod m20261108_000002_add_launches;
mod m20261111_000001_add_worktree_ownership;

use sea_orm_migration::{MigrationTrait, MigratorTrait};

pub(crate) struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20261108_000001_create_link_store::Migration),
            Box::new(m20261108_000002_add_launches::Migration),
            Box::new(m20261111_000001_add_worktree_ownership::Migration),
        ]
    }
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, SqlErr, Statement};
    use sea_orm_migration::{MigrationTrait, MigratorTrait};

    use super::{Migrator, m20261108_000001_create_link_store, m20261108_000002_add_launches};

    struct Previous;

    impl MigratorTrait for Previous {
        fn migrations() -> Vec<Box<dyn MigrationTrait>> {
            vec![
                Box::new(m20261108_000001_create_link_store::Migration),
                Box::new(m20261108_000002_add_launches::Migration),
            ]
        }
    }

    async fn previous_database() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("PRAGMA foreign_keys = ON")
            .await
            .unwrap();
        Previous::up(&db, None).await.unwrap();
        db.execute_unprepared("INSERT INTO sessions (host, native_id, cwd, worktree_path, started_at) VALUES ('codex', 'existing', '/old', '/old/location', '2026-11-10 00:00:00+00:00')")
            .await.unwrap();
        db
    }

    #[tokio::test]
    async fn worktree_ownership_migration_preserves_locations_without_claiming_them() {
        let db = previous_database().await;
        Migrator::up(&db, None).await.unwrap();
        let existing = db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT worktree_path FROM sessions WHERE native_id = 'existing'",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            existing.try_get::<String>("", "worktree_path").unwrap(),
            "/old/location"
        );
        let count = db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT count(*) AS count FROM worktree_ownerships",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(count.try_get::<i64>("", "count").unwrap(), 0);
        let invalid_owner = db.execute_unprepared("INSERT INTO worktree_ownerships (path, session_id, claimed_at) VALUES ('/unowned', 9000, '2026-11-11 00:00:00+00:00')").await.unwrap_err();
        assert!(matches!(
            invalid_owner.sql_err(),
            Some(SqlErr::ForeignKeyConstraintViolation(_))
        ));
        assert!(
            Migrator::get_pending_migrations(&db)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn worktree_ownership_migration_rolls_back_its_table_and_history_on_failure() {
        let db = previous_database().await;
        db.execute_unprepared("CREATE INDEX idx_worktree_ownerships_owner ON sessions(host)")
            .await
            .unwrap();
        assert!(Migrator::up(&db, None).await.is_err());
        let table = db.query_one_raw(Statement::from_string(DbBackend::Sqlite, "SELECT name FROM sqlite_master WHERE name = 'worktree_ownerships' AND type = 'table'")).await.unwrap();
        assert!(table.is_none());
        assert_eq!(
            Migrator::get_pending_migrations(&db).await.unwrap().len(),
            1
        );
        let existing = db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT cwd FROM sessions WHERE native_id = 'existing'",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(existing.try_get::<String>("", "cwd").unwrap(), "/old");
        db.execute_unprepared("DROP INDEX idx_worktree_ownerships_owner")
            .await
            .unwrap();
        Migrator::up(&db, None).await.unwrap();
        assert!(
            Migrator::get_pending_migrations(&db)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
