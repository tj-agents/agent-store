mod entity;
pub mod legacy;
mod migration;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sea_orm::sea_query::OnConflict;
use sea_orm::sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sea_orm::{
    ActiveValue::{NotSet, Set},
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder,
    SqlxSqliteConnector, TransactionTrait,
};
use sea_orm_migration::MigratorTrait;
use thiserror::Error;

use crate::domain::reference::WorkItemRef;
use crate::domain::session::ports::{Item, LinkStore, StoreError};
use crate::domain::session::{
    EndReason, Handle, Host, Link, Registration, Role, Session, State, Terminal, Wrapper,
};
use entity::{session, session_link, work_item_reference};
use migration::Migrator;

const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// The SQLite-backed [`LinkStore`].
pub struct Store {
    db: DatabaseConnection,
}

#[derive(Debug, Error)]
pub enum OpenError {
    #[error("cannot create the database directory {path}")]
    Directory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot open the database {path}")]
    Connect {
        path: PathBuf,
        #[source]
        source: DbErr,
    },
    #[error("cannot migrate the database {path}")]
    Migrate {
        path: PathBuf,
        #[source]
        source: DbErr,
    },
}

#[derive(Debug, Error)]
#[error("stored row is invalid: {0}")]
struct InvalidRow(String);

impl Store {
    /// Opens or creates the database at `path` and applies pending migrations.
    pub async fn open(path: &Path) -> Result<Self, OpenError> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|source| OpenError::Directory {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(BUSY_TIMEOUT)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|source| OpenError::Connect {
                path: path.to_path_buf(),
                source: DbErr::Conn(sea_orm::RuntimeErr::SqlxError(source.into())),
            })?;
        let db = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool);
        migrate(&db).await.map_err(|source| OpenError::Migrate {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(Self { db })
    }

    async fn save_session(&self, session: &Session) -> Result<(), DbErr> {
        let txn = self.db.begin().await?;
        let row = session::Entity::insert(session_row(session))
            .on_conflict(
                OnConflict::columns([session::Column::Host, session::Column::NativeId])
                    .update_columns([
                        session::Column::Cwd,
                        session::Column::WorktreePath,
                        session::Column::TerminalKind,
                        session::Column::TerminalId,
                        session::Column::WrapperPid,
                        session::Column::WrapperStartedAt,
                        session::Column::ClosingRequestedAt,
                        session::Column::EndedAt,
                        session::Column::EndReason,
                    ])
                    .to_owned(),
            )
            .exec_with_returning(&txn)
            .await?;
        for link in session.links() {
            let reference = reference_id(&txn, &link.item).await?;
            session_link::Entity::insert(session_link::ActiveModel {
                session_id: Set(row.id),
                reference_id: Set(reference),
                role: Set(link.role.as_str().to_owned()),
                linked_at: Set(link.linked_at),
            })
            .on_conflict(
                OnConflict::columns([
                    session_link::Column::SessionId,
                    session_link::Column::ReferenceId,
                ])
                .update_column(session_link::Column::Role)
                .to_owned(),
            )
            .exec_without_returning(&txn)
            .await?;
        }
        txn.commit().await
    }

    async fn load(&self, rows: Vec<session::Model>) -> Result<Vec<Session>, StoreError> {
        let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
        let mut links: HashMap<i64, Vec<Link>> = HashMap::new();
        for (link, reference) in session_link::Entity::find()
            .filter(session_link::Column::SessionId.is_in(ids))
            .find_also_related(work_item_reference::Entity)
            .order_by_asc(session_link::Column::LinkedAt)
            .all(&self.db)
            .await
            .map_err(unavailable)?
        {
            let reference = reference.ok_or_else(|| {
                invalid(format!(
                    "link of session {} has no reference",
                    link.session_id
                ))
            })?;
            let session_id = link.session_id;
            links
                .entry(session_id)
                .or_default()
                .push(to_link(&link, &reference)?);
        }
        rows.into_iter()
            .map(|row| {
                let session_links = links.remove(&row.id).unwrap_or_default();
                to_session(row, session_links)
            })
            .collect()
    }
}

impl LinkStore for Store {
    async fn find(&self, handle: &Handle) -> Result<Option<Session>, StoreError> {
        let row = session::Entity::find()
            .filter(session::Column::Host.eq(handle.host().as_str()))
            .filter(session::Column::NativeId.eq(handle.native_id()))
            .one(&self.db)
            .await
            .map_err(unavailable)?;
        Ok(self.load(row.into_iter().collect()).await?.pop())
    }

    async fn save(&self, session: &Session) -> Result<(), StoreError> {
        self.save_session(session).await.map_err(unavailable)
    }

    async fn sessions_for(&self, item: &WorkItemRef) -> Result<Vec<Session>, StoreError> {
        let rows = session::Entity::find()
            .inner_join(session_link::Entity)
            .filter(
                session_link::Column::ReferenceId.in_subquery(
                    sea_orm::sea_query::Query::select()
                        .column(work_item_reference::Column::Id)
                        .from(work_item_reference::Entity)
                        .and_where(
                            work_item_reference::Column::Provider.eq(item.provider().as_str()),
                        )
                        .and_where(work_item_reference::Column::Scope.eq(item.scope().as_str()))
                        .and_where(
                            work_item_reference::Column::NativeId.eq(item.native_id().as_str()),
                        )
                        .to_owned(),
                ),
            )
            .order_by_asc(session::Column::StartedAt)
            .all(&self.db)
            .await
            .map_err(unavailable)?;
        self.load(rows).await
    }

    async fn items(&self) -> Result<Vec<Item>, StoreError> {
        let rows = work_item_reference::Entity::find()
            .find_with_related(session_link::Entity)
            .all(&self.db)
            .await
            .map_err(unavailable)?;
        let handles: HashMap<i64, Handle> = session::Entity::find()
            .all(&self.db)
            .await
            .map_err(unavailable)?
            .into_iter()
            .map(|row| Ok((row.id, handle_of(&row)?)))
            .collect::<Result<_, StoreError>>()?;
        let mut items = rows
            .into_iter()
            .filter(|(_, links)| !links.is_empty())
            .map(|(reference, links)| {
                let reference = to_reference(&reference)?;
                let sessions = links
                    .into_iter()
                    .map(|link| {
                        let handle = handles.get(&link.session_id).cloned().ok_or_else(|| {
                            invalid(format!("link to missing session {}", link.session_id))
                        })?;
                        Ok((handle, parse_role(&link.role)?))
                    })
                    .collect::<Result<_, StoreError>>()?;
                Ok(Item {
                    reference,
                    sessions,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        items.sort_by(|a, b| a.reference.cmp(&b.reference));
        Ok(items)
    }
}

async fn migrate(db: &DatabaseConnection) -> Result<(), DbErr> {
    if Migrator::get_pending_migrations(db).await?.is_empty() {
        return Ok(());
    }
    match Migrator::up(db, None).await {
        Ok(()) => Ok(()),
        // Another process may have applied the same migrations between the check and `up`.
        Err(_) if Migrator::get_pending_migrations(db).await?.is_empty() => Ok(()),
        Err(e) => Err(e),
    }
}

async fn reference_id(db: &impl ConnectionTrait, item: &WorkItemRef) -> Result<i64, DbErr> {
    let row = work_item_reference::Entity::insert(work_item_reference::ActiveModel {
        id: NotSet,
        provider: Set(item.provider().as_str().to_owned()),
        scope: Set(item.scope().as_str().to_owned()),
        native_id: Set(item.native_id().as_str().to_owned()),
    })
    .on_conflict(
        OnConflict::columns([
            work_item_reference::Column::Provider,
            work_item_reference::Column::Scope,
            work_item_reference::Column::NativeId,
        ])
        .update_column(work_item_reference::Column::Provider)
        .to_owned(),
    )
    .exec_with_returning(db)
    .await?;
    Ok(row.id)
}

fn session_row(session: &Session) -> session::ActiveModel {
    let (closing_requested_at, ended_at, end_reason) = match session.state() {
        State::Active => (None, None, None),
        State::Closing { requested_at } => (Some(*requested_at), None, None),
        State::Ended { at, reason } => (None, Some(*at), Some(reason.as_str().to_owned())),
    };
    session::ActiveModel {
        id: NotSet,
        host: Set(session.handle().host().as_str().to_owned()),
        native_id: Set(session.handle().native_id().to_owned()),
        cwd: Set(session.cwd().to_string_lossy().into_owned()),
        worktree_path: Set(session.worktree().map(|p| p.to_string_lossy().into_owned())),
        terminal_kind: Set(session.terminal().map(|t| t.kind().to_owned())),
        terminal_id: Set(session.terminal().and_then(Terminal::id).map(str::to_owned)),
        wrapper_pid: Set(session.wrapper().map(|w| i64::from(w.pid))),
        wrapper_started_at: Set(session.wrapper().map(|w| w.started_at)),
        started_at: Set(session.started_at()),
        closing_requested_at: Set(closing_requested_at),
        ended_at: Set(ended_at),
        end_reason: Set(end_reason),
    }
}

fn handle_of(row: &session::Model) -> Result<Handle, StoreError> {
    let host = Host::parse(&row.host).map_err(|e| invalid(e.to_string()))?;
    Handle::new(host, &row.native_id).map_err(|e| invalid(e.to_string()))
}

fn to_session(row: session::Model, links: Vec<Link>) -> Result<Session, StoreError> {
    let handle = handle_of(&row)?;
    let terminal = row
        .terminal_kind
        .as_deref()
        .map(|kind| Terminal::new(kind, row.terminal_id.as_deref()))
        .transpose()
        .map_err(|e| invalid(e.to_string()))?;
    let wrapper = match (row.wrapper_pid, row.wrapper_started_at) {
        (Some(pid), Some(started_at)) => Some(Wrapper {
            pid: u32::try_from(pid).map_err(|e| invalid(e.to_string()))?,
            started_at,
        }),
        _ => None,
    };
    let state = match (row.ended_at, row.end_reason, row.closing_requested_at) {
        (Some(at), Some(reason), _) => State::Ended {
            at,
            reason: EndReason::parse(&reason).map_err(|e| invalid(e.to_string()))?,
        },
        (None, None, Some(requested_at)) => State::Closing { requested_at },
        (None, None, None) => State::Active,
        _ => return Err(invalid(format!("session {handle} has an inconsistent end"))),
    };
    let registration = Registration {
        handle,
        cwd: row.cwd.into(),
        worktree: row.worktree_path.map(PathBuf::from),
        terminal,
        wrapper,
        started_at: row.started_at,
    };
    Ok(Session::restore(registration, state, links))
}

fn to_reference(row: &work_item_reference::Model) -> Result<WorkItemRef, StoreError> {
    WorkItemRef::new(&row.provider, &row.scope, &row.native_id).map_err(|e| invalid(e.to_string()))
}

fn to_link(
    link: &session_link::Model,
    reference: &work_item_reference::Model,
) -> Result<Link, StoreError> {
    Ok(Link {
        item: to_reference(reference)?,
        role: parse_role(&link.role)?,
        linked_at: link.linked_at,
    })
}

fn parse_role(text: &str) -> Result<Role, StoreError> {
    Role::parse(text).map_err(|e| invalid(e.to_string()))
}

fn unavailable(error: DbErr) -> StoreError {
    StoreError(Box::new(error))
}

fn invalid(detail: String) -> StoreError {
    StoreError(Box::new(InvalidRow(detail)))
}
