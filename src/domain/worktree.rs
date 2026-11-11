pub mod ports;
pub mod service;

use std::path::Component;

use chrono::{DateTime, Utc};
use thiserror::Error;

use super::session::ports::StoreError;
use super::session::{Handle, LaunchId};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Owner {
    pub session: Handle,
    pub launch: Option<LaunchId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ownership {
    pub path: Path,
    pub owner: Owner,
    pub claimed_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum OwnershipError {
    #[error("session {0} is not registered")]
    NotRegistered(Handle),
    #[error("worktree ownership does not match the active session and launch")]
    Binding,
    #[error("worktree {} is owned by {}", .0.path.as_str(), .0.owner.session)]
    Claimed(Ownership),
    #[error("worktree path must be an absolute UTF-8 path without parent components")]
    InvalidPath,
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Path(String);

impl Path {
    pub fn new(path: &std::path::Path) -> Result<Self, OwnershipError> {
        if !path.is_absolute() || path.components().any(|part| part == Component::ParentDir) {
            return Err(OwnershipError::InvalidPath);
        }
        let text = path.to_str().ok_or(OwnershipError::InvalidPath)?;
        Ok(Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
