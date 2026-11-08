use std::error::Error as StdError;
use std::future::Future;

use thiserror::Error;

use super::{Handle, Launch, LaunchId, Role, Session, Title};
use crate::domain::reference::WorkItemRef;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("link store unavailable")]
    Unavailable(#[source] Box<dyn StdError + Send + Sync>),
    #[error("link store holds invalid data: {0}")]
    Corrupt(String),
}

#[derive(Debug, Error)]
#[error("launch {} is already recorded", .0.as_str())]
pub struct LaunchExists(pub LaunchId);

/// A work item and the sessions linked to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub reference: WorkItemRef,
    pub sessions: Vec<(Handle, Role)>,
}

/// Persists sessions with their links, and launches. Each update reads, changes and saves as one unit, so
/// concurrent writers never undo each other.
pub trait LinkStore {
    fn find(
        &self,
        handle: &Handle,
    ) -> impl Future<Output = Result<Option<Session>, StoreError>> + Send;

    /// Saves the session `change` returns from the stored one (`None` when unregistered); an `Err` from
    /// `change` leaves the store unchanged.
    fn update<E: Send>(
        &self,
        handle: &Handle,
        change: impl FnOnce(Option<Session>) -> Result<Session, E> + Send,
    ) -> impl Future<Output = Result<Result<Session, E>, StoreError>> + Send;

    /// Applies the changes to every session linked to `item` and every launch for `item`, as one unit,
    /// and saves those they report as changed.
    fn update_linked(
        &self,
        item: &WorkItemRef,
        change_session: impl FnMut(&mut Session) -> bool + Send,
        change_launch: impl FnMut(&mut Launch) -> bool + Send,
    ) -> impl Future<Output = Result<(Vec<Session>, Vec<Launch>), StoreError>> + Send;

    fn sessions_for(
        &self,
        item: &WorkItemRef,
    ) -> impl Future<Output = Result<Vec<Session>, StoreError>> + Send;

    fn titled(
        &self,
        title: &Title,
    ) -> impl Future<Output = Result<Vec<Session>, StoreError>> + Send;

    fn items(&self) -> impl Future<Output = Result<Vec<Item>, StoreError>> + Send;

    fn create_launch(
        &self,
        launch: Launch,
    ) -> impl Future<Output = Result<Result<Launch, LaunchExists>, StoreError>> + Send;

    fn find_launch(
        &self,
        id: &LaunchId,
    ) -> impl Future<Output = Result<Option<Launch>, StoreError>> + Send;

    /// Saves the state of the launch `change` returns from the stored one (`None` when unrecorded); an
    /// `Err` from `change` leaves the store unchanged.
    fn update_launch<E: Send>(
        &self,
        id: &LaunchId,
        change: impl FnOnce(Option<Launch>) -> Result<Launch, E> + Send,
    ) -> impl Future<Output = Result<Result<Launch, E>, StoreError>> + Send;

    /// Every launch, or every launch for `item`, oldest first.
    fn launches(
        &self,
        item: Option<&WorkItemRef>,
    ) -> impl Future<Output = Result<Vec<Launch>, StoreError>> + Send;

    /// Saves the session `change` builds from the launch and the stored session (each `None` when absent)
    /// and binds it to the launch, as one unit; an `Err` from `change` leaves the store unchanged.
    fn bind<E: Send>(
        &self,
        launch: &LaunchId,
        handle: &Handle,
        change: impl FnOnce(Option<Launch>, Option<Session>) -> Result<Session, E> + Send,
    ) -> impl Future<Output = Result<Result<Session, E>, StoreError>> + Send;
}
