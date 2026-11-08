use std::error::Error as StdError;
use std::future::Future;

use thiserror::Error;

use super::{Handle, Role, Session};
use crate::domain::reference::WorkItemRef;

#[derive(Debug, Error)]
#[error("link store unavailable")]
pub struct StoreError(#[source] pub Box<dyn StdError + Send + Sync>);

/// A work item and the sessions linked to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub reference: WorkItemRef,
    pub sessions: Vec<(Handle, Role)>,
}

/// Persists sessions with their links. `save` is additive for links and never removes one.
pub trait LinkStore {
    fn find(
        &self,
        handle: &Handle,
    ) -> impl Future<Output = Result<Option<Session>, StoreError>> + Send;

    fn save(&self, session: &Session) -> impl Future<Output = Result<(), StoreError>> + Send;

    fn sessions_for(
        &self,
        item: &WorkItemRef,
    ) -> impl Future<Output = Result<Vec<Session>, StoreError>> + Send;

    fn items(&self) -> impl Future<Output = Result<Vec<Item>, StoreError>> + Send;
}
