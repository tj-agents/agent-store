mod entity;
pub(crate) mod legacy;
mod migration;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sea_orm::{
    ActiveValue::{NotSet, Set},
    ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbErr, EntityTrait,
    QueryFilter, QueryOrder, SqliteTransactionMode, SqlxSqliteConnector, TransactionOptions,
    TransactionTrait,
};
use sea_orm_migration::MigratorTrait;
use thiserror::Error;

use crate::domain::reference::WorkItemRef;
use crate::domain::session::ports::{Item, LaunchExists, LinkStore, StoreError};
use crate::domain::session::{
    EndReason, Handle, Host, Launch, LaunchId, Link, Registration, Role, Session, State, Terminal,
    Title, Wrapper,
};
use entity::{launch, session, session_link, work_item_reference};
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

    async fn write(&self) -> Result<DatabaseTransaction, StoreError> {
        self.db
            .begin_with_options(TransactionOptions {
                sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
                ..TransactionOptions::default()
            })
            .await
            .map_err(unavailable)
    }

    /// Saves each session whose handle is not stored yet, in one transaction; returns their indexes.
    pub(crate) async fn insert_absent(
        &self,
        sessions: &[Session],
    ) -> Result<Vec<usize>, StoreError> {
        let txn = self.write().await?;
        let mut inserted = Vec::new();
        for (index, session) in sessions.iter().enumerate() {
            if find_row(&txn, session.handle()).await?.is_none() {
                save(&txn, session).await.map_err(unavailable)?;
                inserted.push(index);
            }
        }
        txn.commit().await.map_err(unavailable)?;
        Ok(inserted)
    }
}

impl LinkStore for Store {
    async fn find(&self, handle: &Handle) -> Result<Option<Session>, StoreError> {
        let row = find_row(&self.db, handle).await?;
        Ok(load(&self.db, row.into_iter().collect()).await?.pop())
    }

    async fn update<E: Send>(
        &self,
        handle: &Handle,
        change: impl FnOnce(Option<Session>) -> Result<Session, E> + Send,
    ) -> Result<Result<Session, E>, StoreError> {
        let txn = self.write().await?;
        let row = find_row(&txn, handle).await?;
        let current = load(&txn, row.into_iter().collect()).await?.pop();
        match change(current) {
            Ok(session) => {
                save(&txn, &session).await.map_err(unavailable)?;
                txn.commit().await.map_err(unavailable)?;
                Ok(Ok(session))
            }
            Err(error) => {
                txn.rollback().await.map_err(unavailable)?;
                Ok(Err(error))
            }
        }
    }

    async fn update_linked(
        &self,
        item: &WorkItemRef,
        mut change_session: impl FnMut(&mut Session) -> bool + Send,
        mut change_launch: impl FnMut(&mut Launch) -> bool + Send,
    ) -> Result<(Vec<Session>, Vec<Launch>), StoreError> {
        let txn = self.write().await?;
        let mut sessions = Vec::new();
        for mut session in linked(&txn, item).await? {
            if change_session(&mut session) {
                save(&txn, &session).await.map_err(unavailable)?;
                sessions.push(session);
            }
        }
        let mut launches = Vec::new();
        for row in launch_rows(&txn, Some(item)).await? {
            let id = row.id;
            let mut launch = to_launch(&txn, row).await?;
            if change_launch(&mut launch) {
                save_launch_state(&txn, id, &launch.state).await?;
                launches.push(launch);
            }
        }
        txn.commit().await.map_err(unavailable)?;
        Ok((sessions, launches))
    }

    async fn sessions_for(&self, item: &WorkItemRef) -> Result<Vec<Session>, StoreError> {
        linked(&self.db, item).await
    }

    async fn titled(&self, title: &Title) -> Result<Vec<Session>, StoreError> {
        let rows = session::Entity::find()
            .filter(session::Column::Title.eq(title.as_str()))
            .order_by_asc(session::Column::StartedAt)
            .all(&self.db)
            .await
            .map_err(unavailable)?;
        load(&self.db, rows).await
    }

    async fn create_launch(
        &self,
        launch: Launch,
    ) -> Result<Result<Launch, LaunchExists>, StoreError> {
        let txn = self.write().await?;
        if find_launch_row(&txn, &launch.id).await?.is_some() {
            txn.rollback().await.map_err(unavailable)?;
            return Ok(Err(LaunchExists(launch.id)));
        }
        let reference = match &launch.item {
            Some(item) => Some(reference_id(&txn, item).await.map_err(unavailable)?),
            None => None,
        };
        launch::Entity::insert(launch_row(&launch, reference))
            .exec_without_returning(&txn)
            .await
            .map_err(unavailable)?;
        txn.commit().await.map_err(unavailable)?;
        Ok(Ok(launch))
    }

    async fn find_launch(&self, id: &LaunchId) -> Result<Option<Launch>, StoreError> {
        match find_launch_row(&self.db, id).await? {
            Some(row) => Ok(Some(to_launch(&self.db, row).await?)),
            None => Ok(None),
        }
    }

    async fn update_launch<E: Send>(
        &self,
        id: &LaunchId,
        change: impl FnOnce(Option<Launch>) -> Result<Launch, E> + Send,
    ) -> Result<Result<Launch, E>, StoreError> {
        let txn = self.write().await?;
        let row = find_launch_row(&txn, id).await?;
        let row_id = row.as_ref().map(|row| row.id);
        let current = match row {
            Some(row) => Some(to_launch(&txn, row).await?),
            None => None,
        };
        match change(current) {
            Ok(launch) => {
                let row_id = row_id.ok_or_else(|| {
                    corrupt(format!(
                        "launch {} was changed before it was recorded",
                        id.as_str()
                    ))
                })?;
                save_launch_state(&txn, row_id, &launch.state).await?;
                txn.commit().await.map_err(unavailable)?;
                Ok(Ok(launch))
            }
            Err(error) => {
                txn.rollback().await.map_err(unavailable)?;
                Ok(Err(error))
            }
        }
    }

    async fn launches(&self, item: Option<&WorkItemRef>) -> Result<Vec<Launch>, StoreError> {
        let mut launches = Vec::new();
        for row in launch_rows(&self.db, item).await? {
            launches.push(to_launch(&self.db, row).await?);
        }
        Ok(launches)
    }

    async fn bind<E: Send>(
        &self,
        launch: &LaunchId,
        handle: &Handle,
        change: impl FnOnce(Option<Launch>, Option<Session>) -> Result<Session, E> + Send,
    ) -> Result<Result<Session, E>, StoreError> {
        let txn = self.write().await?;
        let launch_row = find_launch_row(&txn, launch).await?;
        let found = match launch_row.clone() {
            Some(row) => Some(to_launch(&txn, row).await?),
            None => None,
        };
        let row = find_row(&txn, handle).await?;
        let current = load(&txn, row.into_iter().collect()).await?.pop();
        match change(found, current) {
            Ok(session) => {
                let id = save(&txn, &session).await.map_err(unavailable)?;
                if let Some(row) = launch_row {
                    launch::Entity::update(launch::ActiveModel {
                        id: Set(row.id),
                        session_id: Set(Some(id)),
                        ..Default::default()
                    })
                    .exec(&txn)
                    .await
                    .map_err(unavailable)?;
                }
                txn.commit().await.map_err(unavailable)?;
                Ok(Ok(session))
            }
            Err(error) => {
                txn.rollback().await.map_err(unavailable)?;
                Ok(Err(error))
            }
        }
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
                            corrupt(format!("link to missing session {}", link.session_id))
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
    let mut pending = Migrator::get_pending_migrations(db).await?.len();
    while pending > 0 {
        let Err(error) = Migrator::up(db, None).await else {
            return Ok(());
        };
        // Another process may be applying the same migrations; retry while it makes progress.
        match Migrator::get_pending_migrations(db).await {
            Ok(now) if now.len() < pending => pending = now.len(),
            _ => return Err(error),
        }
    }
    Ok(())
}

async fn find_row(
    db: &impl ConnectionTrait,
    handle: &Handle,
) -> Result<Option<session::Model>, StoreError> {
    session::Entity::find()
        .filter(session::Column::Host.eq(handle.host().as_str()))
        .filter(session::Column::NativeId.eq(handle.native_id()))
        .one(db)
        .await
        .map_err(unavailable)
}

async fn find_launch_row(
    db: &impl ConnectionTrait,
    id: &LaunchId,
) -> Result<Option<launch::Model>, StoreError> {
    launch::Entity::find()
        .filter(launch::Column::LaunchId.eq(id.as_str()))
        .one(db)
        .await
        .map_err(unavailable)
}

async fn launch_rows(
    db: &impl ConnectionTrait,
    item: Option<&WorkItemRef>,
) -> Result<Vec<launch::Model>, StoreError> {
    let mut query = launch::Entity::find().order_by_asc(launch::Column::CreatedAt);
    if let Some(item) = item {
        query = query.filter(launch::Column::ReferenceId.in_subquery(reference_query(item)));
    }
    query.all(db).await.map_err(unavailable)
}

async fn save_launch_state(
    db: &impl ConnectionTrait,
    id: i64,
    state: &State,
) -> Result<(), StoreError> {
    let (closing_requested_at, ended_at, end_reason) = state_columns(state);
    launch::Entity::update(launch::ActiveModel {
        id: Set(id),
        closing_requested_at: Set(closing_requested_at),
        ended_at: Set(ended_at),
        end_reason: Set(end_reason),
        ..Default::default()
    })
    .exec(db)
    .await
    .map_err(unavailable)?;
    Ok(())
}

fn reference_query(item: &WorkItemRef) -> sea_orm::sea_query::SelectStatement {
    sea_orm::sea_query::Query::select()
        .column(work_item_reference::Column::Id)
        .from(work_item_reference::Entity)
        .and_where(work_item_reference::Column::Provider.eq(item.provider().as_str()))
        .and_where(work_item_reference::Column::Scope.eq(item.scope().as_str()))
        .and_where(work_item_reference::Column::NativeId.eq(item.native_id().as_str()))
        .to_owned()
}

async fn linked(db: &impl ConnectionTrait, item: &WorkItemRef) -> Result<Vec<Session>, StoreError> {
    let rows = session::Entity::find()
        .inner_join(session_link::Entity)
        .filter(session_link::Column::ReferenceId.in_subquery(reference_query(item)))
        .order_by_asc(session::Column::StartedAt)
        .all(db)
        .await
        .map_err(unavailable)?;
    load(db, rows).await
}

async fn load(
    db: &impl ConnectionTrait,
    rows: Vec<session::Model>,
) -> Result<Vec<Session>, StoreError> {
    let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
    let mut links: HashMap<i64, Vec<Link>> = HashMap::new();
    for (link, reference) in session_link::Entity::find()
        .filter(session_link::Column::SessionId.is_in(ids))
        .find_also_related(work_item_reference::Entity)
        .order_by_asc(session_link::Column::LinkedAt)
        .all(db)
        .await
        .map_err(unavailable)?
    {
        let reference = reference.ok_or_else(|| {
            corrupt(format!(
                "link of session {} has no reference",
                link.session_id
            ))
        })?;
        links
            .entry(link.session_id)
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

async fn save(db: &impl ConnectionTrait, session: &Session) -> Result<i64, DbErr> {
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
                    session::Column::Title,
                    session::Column::Parent,
                    session::Column::LaunchId,
                ])
                .to_owned(),
        )
        .exec_with_returning(db)
        .await?;
    for link in session.links() {
        let reference = reference_id(db, &link.item).await?;
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
        .exec_without_returning(db)
        .await?;
    }
    Ok(row.id)
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

type StateColumns = (Option<DateTime<Utc>>, Option<DateTime<Utc>>, Option<String>);

fn state_columns(state: &State) -> StateColumns {
    match state {
        State::Active => (None, None, None),
        State::Closing { requested_at } => (Some(*requested_at), None, None),
        State::Ended { at, reason } => (None, Some(*at), Some(reason.as_str().to_owned())),
    }
}

fn state_of(columns: StateColumns, owner: &dyn std::fmt::Display) -> Result<State, StoreError> {
    match columns {
        (None, Some(at), Some(reason)) => Ok(State::Ended {
            at,
            reason: EndReason::parse(&reason).map_err(|e| corrupt(e.to_string()))?,
        }),
        (Some(requested_at), None, None) => Ok(State::Closing { requested_at }),
        (None, None, None) => Ok(State::Active),
        _ => Err(corrupt(format!("{owner} has an inconsistent end"))),
    }
}

fn session_row(session: &Session) -> session::ActiveModel {
    let (closing_requested_at, ended_at, end_reason) = state_columns(session.state());
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
        title: Set(session.title().map(|t| t.as_str().to_owned())),
        parent: Set(session.parent().map(ToString::to_string)),
        launch_id: Set(session.launch().map(|l| l.as_str().to_owned())),
    }
}

fn launch_row(launch: &Launch, reference: Option<i64>) -> launch::ActiveModel {
    launch::ActiveModel {
        id: NotSet,
        launch_id: Set(launch.id.as_str().to_owned()),
        host: Set(launch.host.as_str().to_owned()),
        title: Set(launch.title.as_str().to_owned()),
        cwd: Set(launch.cwd.to_string_lossy().into_owned()),
        worktree_path: Set(launch
            .worktree
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())),
        terminal_kind: Set(launch.terminal.as_ref().map(|t| t.kind().to_owned())),
        terminal_id: Set(launch
            .terminal
            .as_ref()
            .and_then(Terminal::id)
            .map(str::to_owned)),
        wrapper_pid: Set(i64::from(launch.wrapper.pid)),
        wrapper_started_at: Set(launch.wrapper.started_at),
        reference_id: Set(reference),
        parent: Set(launch.parent.as_ref().map(ToString::to_string)),
        created_at: Set(launch.created_at),
        session_id: Set(None),
        closing_requested_at: Set(None),
        ended_at: Set(None),
        end_reason: Set(None),
    }
}

async fn to_launch(db: &impl ConnectionTrait, row: launch::Model) -> Result<Launch, StoreError> {
    let item = match row.reference_id {
        Some(id) => {
            let reference = work_item_reference::Entity::find_by_id(id)
                .one(db)
                .await
                .map_err(unavailable)?
                .ok_or_else(|| {
                    corrupt(format!(
                        "launch {} links a missing reference",
                        row.launch_id
                    ))
                })?;
            Some(to_reference(&reference)?)
        }
        None => None,
    };
    let session = match row.session_id {
        Some(id) => {
            let session = session::Entity::find_by_id(id)
                .one(db)
                .await
                .map_err(unavailable)?
                .ok_or_else(|| {
                    corrupt(format!("launch {} binds a missing session", row.launch_id))
                })?;
            Some(handle_of(&session)?)
        }
        None => None,
    };
    Ok(Launch {
        id: LaunchId::parse(&row.launch_id).map_err(|e| corrupt(e.to_string()))?,
        host: Host::parse(&row.host).map_err(|e| corrupt(e.to_string()))?,
        title: Title::parse(&row.title).map_err(|e| corrupt(e.to_string()))?,
        cwd: row.cwd.into(),
        worktree: row.worktree_path.map(PathBuf::from),
        terminal: row
            .terminal_kind
            .as_deref()
            .map(|kind| Terminal::new(kind, row.terminal_id.as_deref()))
            .transpose()
            .map_err(|e| corrupt(e.to_string()))?,
        wrapper: Wrapper {
            pid: u32::try_from(row.wrapper_pid).map_err(|e| corrupt(e.to_string()))?,
            started_at: row.wrapper_started_at,
        },
        item,
        parent: optional(row.parent.as_deref(), Handle::parse)?,
        created_at: row.created_at,
        session,
        state: state_of(
            (row.closing_requested_at, row.ended_at, row.end_reason),
            &format_args!("launch {}", row.launch_id),
        )?,
    })
}

fn optional<T, E: std::fmt::Display>(
    text: Option<&str>,
    parse: impl FnOnce(&str) -> Result<T, E>,
) -> Result<Option<T>, StoreError> {
    text.map(parse)
        .transpose()
        .map_err(|e| corrupt(e.to_string()))
}

fn handle_of(row: &session::Model) -> Result<Handle, StoreError> {
    let host = Host::parse(&row.host).map_err(|e| corrupt(e.to_string()))?;
    Handle::new(host, &row.native_id).map_err(|e| corrupt(e.to_string()))
}

fn to_session(row: session::Model, links: Vec<Link>) -> Result<Session, StoreError> {
    let handle = handle_of(&row)?;
    let terminal = row
        .terminal_kind
        .as_deref()
        .map(|kind| Terminal::new(kind, row.terminal_id.as_deref()))
        .transpose()
        .map_err(|e| corrupt(e.to_string()))?;
    let wrapper = match (row.wrapper_pid, row.wrapper_started_at) {
        (Some(pid), Some(started_at)) => Some(Wrapper {
            pid: u32::try_from(pid).map_err(|e| corrupt(e.to_string()))?,
            started_at,
        }),
        _ => None,
    };
    let state = state_of(
        (row.closing_requested_at, row.ended_at, row.end_reason),
        &format_args!("session {handle}"),
    )?;
    let registration = Registration {
        handle,
        cwd: row.cwd.into(),
        worktree: row.worktree_path.map(PathBuf::from),
        terminal,
        wrapper,
        title: optional(row.title.as_deref(), Title::parse)?,
        parent: optional(row.parent.as_deref(), Handle::parse)?,
        launch: optional(row.launch_id.as_deref(), LaunchId::parse)?,
        started_at: row.started_at,
    };
    Ok(Session::restore(registration, state, links))
}

fn to_reference(row: &work_item_reference::Model) -> Result<WorkItemRef, StoreError> {
    WorkItemRef::new(&row.provider, &row.scope, &row.native_id).map_err(|e| corrupt(e.to_string()))
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
    Role::parse(text).map_err(|e| corrupt(e.to_string()))
}

fn unavailable(error: DbErr) -> StoreError {
    StoreError::Unavailable(Box::new(error))
}

fn corrupt(detail: String) -> StoreError {
    StoreError::Corrupt(detail)
}
