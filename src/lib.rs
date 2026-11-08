//! A link store between tracker work items and agent host sessions, kept in `SQLite`.
//!
//! It records which sessions work on which work items, and where each session runs. It does not
//! record tracker state or whether a session's process is alive; read those from their owners.
//!
//! ```no_run
//! use std::path::Path;
//!
//! use agent_store::domain::reference::WorkItemRef;
//! use agent_store::domain::session::{Handle, Registration, Role, service};
//! use agent_store::sqlite::Store;
//!
//! async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//!     let store = Store::open(Path::new("links.db")).await?;
//!     let handle = Handle::parse("claude:abc")?;
//!     let registration = Registration {
//!         handle: handle.clone(),
//!         cwd: "/src/app".into(),
//!         worktree: None,
//!         terminal: None,
//!         wrapper: None,
//!         started_at: chrono::Utc::now(),
//!     };
//!     service::register(&store, registration).await?;
//!     let item = WorkItemRef::parse("github:github.com/acme/app:461", None)?;
//!     let owner = Role::parse(Role::OWNER)?;
//!     service::link(&store, &handle, item.clone(), owner, chrono::Utc::now()).await?;
//!     for session in service::sessions(&store, &item).await? {
//!         println!("{}", session.handle());
//!     }
//!     Ok(())
//! }
//! ```

pub mod cli;
pub mod domain;
mod git;
pub mod sqlite;
