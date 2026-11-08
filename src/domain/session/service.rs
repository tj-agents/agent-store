use std::convert::Infallible;

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
    let handle = registration.handle.clone();
    let session = store
        .update(&handle, |existing| {
            Ok::<_, Infallible>(match existing {
                Some(mut session) => {
                    session.resume(registration);
                    session
                }
                None => Session::register(registration),
            })
        })
        .await?;
    Ok(session.unwrap_or_else(|never| match never {}))
}

pub async fn link(
    store: &impl LinkStore,
    handle: &Handle,
    item: WorkItemRef,
    role: Role,
    now: DateTime<Utc>,
) -> Result<Session, LinkError> {
    store
        .update(handle, |session| {
            let mut session = registered(session, handle)?;
            session.link(item, role, now);
            Ok(session)
        })
        .await
        .map_err(LinkError::Store)?
        .map_err(LinkError::NotRegistered)
}

pub async fn end(
    store: &impl LinkStore,
    handle: &Handle,
    reason: EndReason,
    now: DateTime<Utc>,
) -> Result<Session, EndError> {
    store
        .update(handle, |session| {
            let mut session = registered(session, handle).map_err(EndError::NotRegistered)?;
            session.end(reason, now).map_err(EndError::AlreadyEnded)?;
            Ok(session)
        })
        .await
        .map_err(EndError::Store)?
}

/// Asks one session's wrapper to close it.
pub async fn mark_session_closing(
    store: &impl LinkStore,
    handle: &Handle,
    now: DateTime<Utc>,
) -> Result<Session, EndError> {
    store
        .update(handle, |session| {
            let mut session = registered(session, handle).map_err(EndError::NotRegistered)?;
            session.request_close(now).map_err(EndError::AlreadyEnded)?;
            Ok(session)
        })
        .await
        .map_err(EndError::Store)?
}

/// Asks the wrapper of every session linked to `item` that has not ended to close it.
pub async fn mark_item_closing(
    store: &impl LinkStore,
    item: &WorkItemRef,
    now: DateTime<Utc>,
) -> Result<Vec<Session>, StoreError> {
    store
        .update_linked(item, |session| {
            !session.is_closing() && session.request_close(now).is_ok()
        })
        .await
}

pub async fn show(store: &impl LinkStore, handle: &Handle) -> Result<Session, ShowError> {
    registered(store.find(handle).await.map_err(ShowError::Store)?, handle)
        .map_err(ShowError::NotRegistered)
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

fn registered(session: Option<Session>, handle: &Handle) -> Result<Session, NotRegistered> {
    session.ok_or_else(|| NotRegistered(handle.clone()))
}
