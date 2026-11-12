use std::convert::Infallible;

use chrono::{DateTime, Utc};
use thiserror::Error;

use super::ports::{Item, LaunchExists, LinkStore, StoreError};
use super::{
    AlreadyEnded, EndReason, Handle, Host, Launch, LaunchEnded, LaunchId, Registration, Role,
    Session, Title,
};
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

#[derive(Debug, Error)]
#[error("launch {} is not recorded", .0.as_str())]
pub struct UnknownLaunch(pub LaunchId);

#[derive(Debug, Error)]
pub enum LaunchError {
    #[error(transparent)]
    Exists(LaunchExists),
    #[error(transparent)]
    Store(StoreError),
}

#[derive(Debug, Error)]
pub enum RegisterError {
    #[error(transparent)]
    UnknownLaunch(UnknownLaunch),
    #[error("launch {} is for host {}, not {}", .launch.as_str(), .expected.as_str(), .actual.as_str())]
    HostMismatch {
        launch: LaunchId,
        expected: Host,
        actual: Host,
    },
    #[error(transparent)]
    Store(StoreError),
}

#[derive(Debug, Error)]
pub enum LaunchStateError {
    #[error(transparent)]
    UnknownLaunch(UnknownLaunch),
    #[error(transparent)]
    Ended(LaunchEnded),
    #[error(transparent)]
    Store(StoreError),
}

#[derive(Debug, Error)]
pub enum SetOpenError {
    #[error(transparent)]
    UnknownLaunch(UnknownLaunch),
    #[error(transparent)]
    Store(StoreError),
}

#[derive(Debug, Error)]
pub enum ShowLaunchError {
    #[error(transparent)]
    UnknownLaunch(UnknownLaunch),
    #[error(transparent)]
    Store(StoreError),
}

/// Records a tab launched for an agent whose host session does not exist yet.
pub async fn launch(store: &impl LinkStore, launch: Launch) -> Result<Launch, LaunchError> {
    store
        .create_launch(launch)
        .await
        .map_err(LaunchError::Store)?
        .map_err(LaunchError::Exists)
}

/// Registers a session started by a launch: it inherits what its registration leaves unset from the
/// launch, is linked to the launch's work item, and becomes the session the launch shows.
pub async fn register_launched(
    store: &impl LinkStore,
    registration: Registration,
    launch: &LaunchId,
    now: DateTime<Utc>,
) -> Result<Session, RegisterError> {
    let handle = registration.handle.clone();
    store
        .bind(launch, &handle, |found, existing| {
            let found =
                found.ok_or_else(|| RegisterError::UnknownLaunch(UnknownLaunch(launch.clone())))?;
            if found.host != *handle.host() {
                return Err(RegisterError::HostMismatch {
                    launch: launch.clone(),
                    expected: found.host,
                    actual: handle.host().clone(),
                });
            }
            let registration = registration.inherit(&found);
            let mut session = match existing {
                Some(mut session) => {
                    session.resume(registration);
                    session
                }
                None => Session::register(registration),
            };
            if let Some(item) = found.item {
                session.link_if_absent(item, Role::owner(), now);
            }
            Ok(session)
        })
        .await
        .map_err(RegisterError::Store)?
}

/// Asks a launch's wrapper to close its agent.
pub async fn mark_launch_closing(
    store: &impl LinkStore,
    id: &LaunchId,
    now: DateTime<Utc>,
) -> Result<Launch, LaunchStateError> {
    store
        .update_launch(id, |launch| {
            let mut launch = recorded(launch, id)?;
            launch.request_close(now).map_err(LaunchStateError::Ended)?;
            Ok(launch)
        })
        .await
        .map_err(LaunchStateError::Store)?
}

/// Records that a launch's wrapper has finished.
pub async fn end_launch(
    store: &impl LinkStore,
    id: &LaunchId,
    reason: EndReason,
    now: DateTime<Utc>,
) -> Result<Launch, LaunchStateError> {
    store
        .update_launch(id, |launch| {
            let mut launch = recorded(launch, id)?;
            launch.end(reason, now).map_err(LaunchStateError::Ended)?;
            Ok(launch)
        })
        .await
        .map_err(LaunchStateError::Store)?
}

/// Records whether a launch's tab is open, whatever the launch's state.
pub async fn set_open(
    store: &impl LinkStore,
    id: &LaunchId,
    open: bool,
) -> Result<Launch, SetOpenError> {
    store
        .update_launch(id, |launch| {
            let mut launch =
                launch.ok_or_else(|| SetOpenError::UnknownLaunch(UnknownLaunch(id.clone())))?;
            launch.open = open;
            Ok(launch)
        })
        .await
        .map_err(SetOpenError::Store)?
}

pub async fn launches(
    store: &impl LinkStore,
    item: Option<&WorkItemRef>,
) -> Result<Vec<Launch>, StoreError> {
    store.launches(item).await
}

fn recorded(launch: Option<Launch>, id: &LaunchId) -> Result<Launch, LaunchStateError> {
    launch.ok_or_else(|| LaunchStateError::UnknownLaunch(UnknownLaunch(id.clone())))
}

/// A launch and the session bound to it, if one has registered.
pub async fn show_launch(
    store: &impl LinkStore,
    id: &LaunchId,
) -> Result<(Launch, Option<Session>), ShowLaunchError> {
    let launch = store
        .find_launch(id)
        .await
        .map_err(ShowLaunchError::Store)?
        .ok_or_else(|| ShowLaunchError::UnknownLaunch(UnknownLaunch(id.clone())))?;
    let session = match &launch.session {
        Some(handle) => store.find(handle).await.map_err(ShowLaunchError::Store)?,
        None => None,
    };
    Ok((launch, session))
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

/// Asks the wrapper of every session linked to `item`, and of every launch for `item`, that has not ended
/// to close it.
pub async fn mark_item_closing(
    store: &impl LinkStore,
    item: &WorkItemRef,
    now: DateTime<Utc>,
) -> Result<(Vec<Session>, Vec<Launch>), StoreError> {
    store
        .update_linked(
            item,
            |session| !session.is_closing() && session.request_close(now).is_ok(),
            |launch| !launch.state.is_closing() && launch.request_close(now).is_ok(),
        )
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

pub async fn titled(store: &impl LinkStore, title: &Title) -> Result<Vec<Session>, StoreError> {
    store.titled(title).await
}

pub async fn all_sessions(store: &impl LinkStore) -> Result<Vec<Session>, StoreError> {
    store.all_sessions().await
}

pub async fn items(store: &impl LinkStore) -> Result<Vec<Item>, StoreError> {
    store.items().await
}

fn registered(session: Option<Session>, handle: &Handle) -> Result<Session, NotRegistered> {
    session.ok_or_else(|| NotRegistered(handle.clone()))
}
