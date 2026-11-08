use std::error::Error as StdError;
use std::future::Future;

use thiserror::Error;

use super::{Handle, Role, Session};
use crate::domain::reference::WorkItemRef;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("link store unavailable")]
    Unavailable(#[source] Box<dyn StdError + Send + Sync>),
    #[error("link store holds invalid data: {0}")]
    Corrupt(String),
}

/// A work item and the sessions linked to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub reference: WorkItemRef,
    pub sessions: Vec<(Handle, Role)>,
}

/// Persists sessions with their links. Each update reads, changes and saves as one unit, so concurrent
/// writers never undo each other.
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

    /// Applies `change` to every session linked to `item` and saves those it reports as changed.
    fn update_linked(
        &self,
        item: &WorkItemRef,
        change: impl FnMut(&mut Session) -> bool + Send,
    ) -> impl Future<Output = Result<Vec<Session>, StoreError>> + Send;

    fn sessions_for(
        &self,
        item: &WorkItemRef,
    ) -> impl Future<Output = Result<Vec<Session>, StoreError>> + Send;

    fn items(&self) -> impl Future<Output = Result<Vec<Item>, StoreError>> + Send;
}
