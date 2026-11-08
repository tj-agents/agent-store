use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use sea_orm::sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sea_orm::{DbBackend, DbErr, FromQueryResult, SqlxSqliteConnector, Statement};
use thiserror::Error;

use super::Store;
use crate::domain::reference::WorkItemRef;
use crate::domain::session::ports::StoreError;
use crate::domain::session::{EndReason, Handle, Registration, Role, Session};

const LEGACY_SCHEMA_VERSION: i64 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImportSummary {
    pub(crate) imported: usize,
    pub(crate) already_present: usize,
    pub(crate) incomplete: usize,
    pub(crate) links: usize,
    pub(crate) unlinked_references: usize,
}

#[derive(Debug, Error)]
pub(crate) enum ImportError {
    #[error("legacy database {0} does not exist")]
    Missing(PathBuf),
    #[error("cannot read the legacy database {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: DbErr,
    },
    #[error("legacy database {path} has schema version {found}, expected {LEGACY_SCHEMA_VERSION}")]
    SchemaVersion { path: PathBuf, found: i64 },
    #[error("legacy row is invalid: {0}")]
    InvalidRow(String),
    #[error(transparent)]
    Store(StoreError),
}

#[derive(FromQueryResult)]
struct LegacyReference {
    provider: String,
    scope: String,
    native_id: String,
    work_item: String,
}

#[derive(FromQueryResult)]
struct LegacySession {
    handle: String,
    cwd: String,
    work_item: Option<String>,
    role: Option<String>,
    started_at_micros: i64,
    ended_at_micros: Option<i64>,
    end_reason: Option<String>,
}

#[derive(FromQueryResult)]
struct Count {
    count: i64,
}

#[derive(FromQueryResult)]
struct UserVersion {
    user_version: i64,
}

/// Imports sessions and their links from the retired Python store (`sessions.db`, schema 4) in one
/// transaction, skipping sessions the store already holds.
pub(crate) async fn import(store: &Store, path: &Path) -> Result<ImportSummary, ImportError> {
    if !path.is_file() {
        return Err(ImportError::Missing(path.to_path_buf()));
    }
    let read = |source| ImportError::Read {
        path: path.to_path_buf(),
        source,
    };
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(path).read_only(true))
        .await
        .map_err(|e| read(DbErr::Custom(e.to_string())))?;
    let legacy = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool);

    let version = UserVersion::find_by_statement(statement("PRAGMA user_version"))
        .one(&legacy)
        .await
        .map_err(read)?
        .map_or(0, |row| row.user_version);
    if version != LEGACY_SCHEMA_VERSION {
        return Err(ImportError::SchemaVersion {
            path: path.to_path_buf(),
            found: version,
        });
    }

    let references = LegacyReference::find_by_statement(statement(
        "SELECT provider, scope, native_id, work_item FROM work_item_references",
    ))
    .all(&legacy)
    .await
    .map_err(read)?;
    let sessions = LegacySession::find_by_statement(statement(
        "SELECT handle, cwd, work_item, role,
                CAST(started_at * 1000000 AS INTEGER) AS started_at_micros,
                CAST(ended_at * 1000000 AS INTEGER) AS ended_at_micros,
                end_reason
         FROM sessions WHERE cwd IS NOT NULL AND started_at IS NOT NULL",
    ))
    .all(&legacy)
    .await
    .map_err(read)?;
    let incomplete = Count::find_by_statement(statement(
        "SELECT COUNT(*) AS count FROM sessions WHERE cwd IS NULL OR started_at IS NULL",
    ))
    .one(&legacy)
    .await
    .map_err(read)?
    .map_or(0, |row| row.count);
    legacy.close_by_ref().await.map_err(read)?;

    let mut by_work_item: HashMap<&str, Vec<WorkItemRef>> = HashMap::new();
    for reference in &references {
        let item = WorkItemRef::new(&reference.provider, &reference.scope, &reference.native_id)
            .map_err(|e| ImportError::InvalidRow(e.to_string()))?;
        by_work_item
            .entry(&reference.work_item)
            .or_default()
            .push(item);
    }

    let converted = sessions
        .iter()
        .map(|row| to_session(row, &by_work_item))
        .collect::<Result<Vec<_>, _>>()?;
    let inserted = store
        .insert_absent(&converted)
        .await
        .map_err(ImportError::Store)?;
    let linked_work_items: HashSet<&str> = sessions
        .iter()
        .filter_map(|row| row.work_item.as_deref())
        .collect();
    Ok(ImportSummary {
        imported: inserted.len(),
        already_present: converted.len() - inserted.len(),
        incomplete: usize::try_from(incomplete).unwrap_or_default(),
        links: inserted.iter().map(|&i| converted[i].links().len()).sum(),
        unlinked_references: by_work_item
            .iter()
            .filter(|(key, _)| !linked_work_items.contains(*key))
            .map(|(_, items)| items.len())
            .sum(),
    })
}

fn to_session(
    row: &LegacySession,
    by_work_item: &HashMap<&str, Vec<WorkItemRef>>,
) -> Result<Session, ImportError> {
    let invalid = |detail: String| ImportError::InvalidRow(format!("{}: {detail}", row.handle));
    let handle = Handle::parse(&row.handle).map_err(|e| invalid(e.to_string()))?;
    let started_at = instant(row.started_at_micros).ok_or_else(|| invalid("started_at".into()))?;
    let mut session = Session::register(Registration {
        handle,
        cwd: row.cwd.clone().into(),
        worktree: None,
        terminal: None,
        wrapper: None,
        started_at,
    });
    let role = legacy_role(row.role.as_deref());
    for item in row
        .work_item
        .as_deref()
        .and_then(|key| by_work_item.get(key))
        .into_iter()
        .flatten()
    {
        session.link(item.clone(), role.clone(), started_at);
    }
    if let Some(ended_at) = row.ended_at_micros {
        let at = instant(ended_at).ok_or_else(|| invalid("ended_at".into()))?;
        let reason = EndReason::parse(row.end_reason.as_deref().unwrap_or("ended"))
            .map_err(|e| invalid(e.to_string()))?;
        session
            .end(reason, at)
            .map_err(|e| invalid(e.to_string()))?;
    }
    Ok(session)
}

fn legacy_role(role: Option<&str>) -> Role {
    role.and_then(|role| Role::parse(role).ok())
        .unwrap_or_else(|| Role::parse("helper").expect("`helper` is a valid role"))
}

fn instant(micros: i64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp_micros(micros)
}

fn statement(sql: &str) -> Statement {
    Statement::from_string(DbBackend::Sqlite, sql)
}
