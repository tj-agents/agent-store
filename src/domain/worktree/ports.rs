use std::future::Future;

use crate::domain::session::ports::StoreError;
use crate::domain::session::{Launch, Session};

use super::{Owner, Ownership, Path};

pub trait Store {
    fn update_ownership<E: Send>(
        &self,
        owner: &Owner,
        path: &Path,
        change: impl FnOnce(
            Option<Session>,
            Option<Launch>,
            Option<Ownership>,
        ) -> Result<Option<Ownership>, E>
        + Send,
    ) -> impl Future<Output = Result<Result<Option<Ownership>, E>, StoreError>> + Send;

    fn worktrees(
        &self,
        owner: &Owner,
    ) -> impl Future<Output = Result<Vec<Ownership>, StoreError>> + Send;
}
