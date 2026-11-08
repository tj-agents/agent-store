pub mod ports;
pub mod service;

use std::fmt;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use thiserror::Error;

use super::reference::WorkItemRef;
use super::slug::{self, Slug, token};

/// The agent host that owns a session, such as `claude` or `codex`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Host(Slug);

impl Host {
    pub const CLAUDE: &str = "claude";
    pub const CODEX: &str = "codex";

    pub fn parse(text: &str) -> Result<Self, slug::ParseError> {
        Slug::parse(text).map(Self)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// A session's identity: `host:native_id`, where the host assigned `native_id`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Handle {
    host: Host,
    native_id: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HandleError {
    #[error("`{0}` is not `host:session-id`")]
    Shape(String),
    #[error("invalid host")]
    Host(#[source] slug::ParseError),
}

impl Handle {
    pub fn new(host: Host, native_id: &str) -> Result<Self, HandleError> {
        let native_id = token(native_id)
            .ok_or_else(|| HandleError::Shape(format!("{}:{native_id}", host.as_str())))?;
        Ok(Self { host, native_id })
    }

    pub fn parse(text: &str) -> Result<Self, HandleError> {
        let (host, native_id) = text
            .split_once(':')
            .ok_or_else(|| HandleError::Shape(text.to_owned()))?;
        Self::new(Host::parse(host).map_err(HandleError::Host)?, native_id)
    }

    pub fn host(&self) -> &Host {
        &self.host
    }

    pub fn native_id(&self) -> &str {
        &self.native_id
    }
}

impl fmt::Display for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host.as_str(), self.native_id)
    }
}

/// The terminal tab or pane a session runs in: a kind such as `windows-terminal` or `tmux`, and its id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Terminal {
    kind: Slug,
    id: Option<String>,
}

impl Terminal {
    pub fn new(kind: &str, id: Option<&str>) -> Result<Self, slug::ParseError> {
        Ok(Self {
            kind: Slug::parse(kind)?,
            id: id.map(str::to_owned),
        })
    }

    pub fn kind(&self) -> &str {
        self.kind.as_str()
    }

    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }
}

/// The process that owns the session's tab, identified by pid and start time against pid reuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Wrapper {
    pub pid: u32,
    pub started_at: DateTime<Utc>,
}

/// A session's advisory role on a work item, such as `owner` or `reviewer`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Role(Slug);

impl Role {
    pub const OWNER: &str = "owner";

    pub fn parse(text: &str) -> Result<Self, slug::ParseError> {
        Slug::parse(text).map(Self)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Why a session ended, such as `closed` or `handed_off`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EndReason(Slug);

impl EndReason {
    pub fn parse(text: &str) -> Result<Self, slug::ParseError> {
        Slug::parse(text).map(Self)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub item: WorkItemRef,
    pub role: Role,
    pub linked_at: DateTime<Utc>,
}

/// Recorded lifecycle only. Whether the process is alive is read from the host, never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Active,
    Closing {
        requested_at: DateTime<Utc>,
    },
    Ended {
        at: DateTime<Utc>,
        reason: EndReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    pub handle: Handle,
    pub cwd: PathBuf,
    pub worktree: Option<PathBuf>,
    pub terminal: Option<Terminal>,
    pub wrapper: Option<Wrapper>,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("session {0} has already ended")]
pub struct AlreadyEnded(pub Handle);

/// A host session and the work items it is linked to.
#[derive(Debug, Clone)]
pub struct Session {
    handle: Handle,
    cwd: PathBuf,
    worktree: Option<PathBuf>,
    terminal: Option<Terminal>,
    wrapper: Option<Wrapper>,
    started_at: DateTime<Utc>,
    state: State,
    links: Vec<Link>,
}

impl Session {
    pub fn register(registration: Registration) -> Self {
        Self {
            handle: registration.handle,
            cwd: registration.cwd,
            worktree: registration.worktree,
            terminal: registration.terminal,
            wrapper: registration.wrapper,
            started_at: registration.started_at,
            state: State::Active,
            links: Vec::new(),
        }
    }

    /// Rebuilds a stored session. Only a store adapter calls this.
    pub fn restore(registration: Registration, state: State, links: Vec<Link>) -> Self {
        Self {
            state,
            links,
            ..Self::register(registration)
        }
    }

    /// A resumed session re-registers: its location and process change and its links stay. An ended
    /// session becomes active again; a pending close request survives.
    pub fn resume(&mut self, registration: Registration) {
        self.cwd = registration.cwd;
        self.worktree = registration.worktree;
        self.terminal = registration.terminal;
        self.wrapper = registration.wrapper;
        if let State::Ended { .. } = self.state {
            self.state = State::Active;
        }
    }

    pub fn link(&mut self, item: WorkItemRef, role: Role, now: DateTime<Utc>) {
        match self.links.iter_mut().find(|link| link.item == item) {
            Some(link) => link.role = role,
            None => self.links.push(Link {
                item,
                role,
                linked_at: now,
            }),
        }
    }

    pub fn request_close(&mut self, now: DateTime<Utc>) -> Result<(), AlreadyEnded> {
        match self.state {
            State::Active => {
                self.state = State::Closing { requested_at: now };
                Ok(())
            }
            State::Closing { .. } => Ok(()),
            State::Ended { .. } => Err(AlreadyEnded(self.handle.clone())),
        }
    }

    pub fn end(&mut self, reason: EndReason, now: DateTime<Utc>) -> Result<(), AlreadyEnded> {
        if let State::Ended { .. } = self.state {
            return Err(AlreadyEnded(self.handle.clone()));
        }
        self.state = State::Ended { at: now, reason };
        Ok(())
    }

    pub fn handle(&self) -> &Handle {
        &self.handle
    }

    pub fn cwd(&self) -> &PathBuf {
        &self.cwd
    }

    pub fn worktree(&self) -> Option<&PathBuf> {
        self.worktree.as_ref()
    }

    pub fn terminal(&self) -> Option<&Terminal> {
        self.terminal.as_ref()
    }

    pub fn wrapper(&self) -> Option<Wrapper> {
        self.wrapper
    }

    pub fn started_at(&self) -> DateTime<Utc> {
        self.started_at
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn is_closing(&self) -> bool {
        matches!(self.state, State::Closing { .. })
    }

    pub fn links(&self) -> &[Link] {
        &self.links
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 8, 12, 0, second).unwrap()
    }

    fn registration(cwd: &str) -> Registration {
        Registration {
            handle: Handle::parse("claude:abc").unwrap(),
            cwd: cwd.into(),
            worktree: None,
            terminal: None,
            wrapper: None,
            started_at: at(0),
        }
    }

    fn item(id: &str) -> WorkItemRef {
        WorkItemRef::new("github", "github.com/a/b", id).unwrap()
    }

    #[test]
    fn handle_round_trips_and_keeps_colons_in_the_native_id() {
        let handle = Handle::parse("codex:01a1:x").unwrap();
        assert_eq!(handle.host().as_str(), "codex");
        assert_eq!(handle.native_id(), "01a1:x");
        assert_eq!(handle.to_string(), "codex:01a1:x");
        assert!(Handle::parse("claude").is_err());
        assert!(Handle::parse("claude:").is_err());
    }

    #[test]
    fn relinking_an_item_updates_its_role_only() {
        let mut session = Session::register(registration("/a"));
        session.link(item("1"), Role::parse("owner").unwrap(), at(1));
        session.link(item("1"), Role::parse("reviewer").unwrap(), at(2));
        assert_eq!(session.links().len(), 1);
        assert_eq!(session.links()[0].role.as_str(), "reviewer");
        assert_eq!(session.links()[0].linked_at, at(1));
    }

    #[test]
    fn close_then_end_then_refuse() {
        let mut session = Session::register(registration("/a"));
        session.request_close(at(1)).unwrap();
        session.request_close(at(2)).unwrap();
        assert_eq!(
            session.state(),
            &State::Closing {
                requested_at: at(1)
            }
        );
        session
            .end(EndReason::parse("closed").unwrap(), at(3))
            .unwrap();
        assert!(session.request_close(at(4)).is_err());
        assert!(
            session
                .end(EndReason::parse("closed").unwrap(), at(4))
                .is_err()
        );
    }

    #[test]
    fn resume_reactivates_and_keeps_links() {
        let mut session = Session::register(registration("/a"));
        session.link(item("1"), Role::parse("owner").unwrap(), at(1));
        session
            .end(EndReason::parse("closed").unwrap(), at(2))
            .unwrap();
        session.resume(registration("/b"));
        assert_eq!(session.state(), &State::Active);
        assert_eq!(session.cwd(), &PathBuf::from("/b"));
        assert_eq!(session.links().len(), 1);
    }

    #[test]
    fn resume_keeps_a_pending_close_request() {
        let mut session = Session::register(registration("/a"));
        session.request_close(at(1)).unwrap();
        session.resume(registration("/a"));
        assert_eq!(
            session.state(),
            &State::Closing {
                requested_at: at(1)
            }
        );
    }
}
