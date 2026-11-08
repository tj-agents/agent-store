use chrono::{DateTime, Utc};
use thiserror::Error;

use super::ports::{Item, LinkStore, StoreError};
use super::{AlreadyEnded, EndReason, Handle, Registration, Role, Session};
use crate::domain::reference::WorkItemRef;

#[derive(Debug, Error)]
#[error("session {0} is not registered")]
pub struct NotRegistered(pub Handle);

#[derive(Debug, Error)]
pub enum LinkError {
    #[error(transparent)]
    NotRegistered(NotRegistered),
    #[error(transparent)]
    Store(StoreError),
}

#[derive(Debug, Error)]
pub enum EndError {
    #[error(transparent)]
    NotRegistered(NotRegistered),
    #[error(transparent)]
    AlreadyEnded(AlreadyEnded),
    #[error(transparent)]
    Store(StoreError),
}

#[derive(Debug, Error)]
pub enum ShowError {
    #[error(transparent)]
    NotRegistered(NotRegistered),
    #[error(transparent)]
    Store(StoreError),
}

/// Records a new session, or re-registers a resumed one.
pub async fn register(
    store: &impl LinkStore,
    registration: Registration,
) -> Result<Session, StoreError> {
    let session = match store.find(&registration.handle).await? {
        Some(mut session) => {
            session.resume(registration);
            session
        }
        None => Session::register(registration),
    };
    store.save(&session).await?;
    Ok(session)
}

pub async fn link(
    store: &impl LinkStore,
    handle: &Handle,
    item: WorkItemRef,
    role: Role,
    now: DateTime<Utc>,
) -> Result<Session, LinkError> {
    let mut session = registered(store, handle)
        .await
        .map_err(|e| e.either(LinkError::NotRegistered, LinkError::Store))?;
    session.link(item, role, now);
    store.save(&session).await.map_err(LinkError::Store)?;
    Ok(session)
}

pub async fn end(
    store: &impl LinkStore,
    handle: &Handle,
    reason: EndReason,
    now: DateTime<Utc>,
) -> Result<Session, EndError> {
    let mut session = registered(store, handle)
        .await
        .map_err(|e| e.either(EndError::NotRegistered, EndError::Store))?;
    session.end(reason, now).map_err(EndError::AlreadyEnded)?;
    store.save(&session).await.map_err(EndError::Store)?;
    Ok(session)
}

/// Asks one session's wrapper to close it.
pub async fn mark_session_closing(
    store: &impl LinkStore,
    handle: &Handle,
    now: DateTime<Utc>,
) -> Result<Session, EndError> {
    let mut session = registered(store, handle)
        .await
        .map_err(|e| e.either(EndError::NotRegistered, EndError::Store))?;
    session.request_close(now).map_err(EndError::AlreadyEnded)?;
    store.save(&session).await.map_err(EndError::Store)?;
    Ok(session)
}

/// Asks the wrapper of every session linked to `item` that has not ended to close it.
pub async fn mark_item_closing(
    store: &impl LinkStore,
    item: &WorkItemRef,
    now: DateTime<Utc>,
) -> Result<Vec<Session>, StoreError> {
    let mut marked = Vec::new();
    for mut session in store.sessions_for(item).await? {
        if session.request_close(now).is_ok() {
            store.save(&session).await?;
            marked.push(session);
        }
    }
    Ok(marked)
}

pub async fn show(store: &impl LinkStore, handle: &Handle) -> Result<Session, ShowError> {
    registered(store, handle)
        .await
        .map_err(|e| e.either(ShowError::NotRegistered, ShowError::Store))
}

pub async fn sessions(
    store: &impl LinkStore,
    item: &WorkItemRef,
) -> Result<Vec<Session>, StoreError> {
    store.sessions_for(item).await
}

pub async fn items(store: &impl LinkStore) -> Result<Vec<Item>, StoreError> {
    store.items().await
}

enum Lookup {
    NotRegistered(NotRegistered),
    Store(StoreError),
}

impl Lookup {
    fn either<E>(
        self,
        missing: impl FnOnce(NotRegistered) -> E,
        store: impl FnOnce(StoreError) -> E,
    ) -> E {
        match self {
            Self::NotRegistered(e) => missing(e),
            Self::Store(e) => store(e),
        }
    }
}

async fn registered(store: &impl LinkStore, handle: &Handle) -> Result<Session, Lookup> {
    store
        .find(handle)
        .await
        .map_err(Lookup::Store)?
        .ok_or_else(|| Lookup::NotRegistered(NotRegistered(handle.clone())))
}
