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

    pub fn owner() -> Self {
        Self(Slug::parse(Self::OWNER).expect("owner is a valid role"))
    }

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

/// The tab title a launcher gave a session, by which people and other sessions address it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Title(String);

#[derive(Debug, Error, PartialEq, Eq)]
#[error("a title must have visible text")]
pub struct TitleError;

impl Title {
    pub fn parse(text: &str) -> Result<Self, TitleError> {
        if text.trim().is_empty() {
            Err(TitleError)
        } else {
            Ok(Self(text.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The id a launcher chose for one tab launch, known before the host assigns a session id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LaunchId(String);

#[derive(Debug, Error, PartialEq, Eq)]
#[error("`{0}` is not a launch id: it must be non-empty with no whitespace")]
pub struct LaunchIdError(pub String);

impl LaunchId {
    pub fn parse(text: &str) -> Result<Self, LaunchIdError> {
        token(text)
            .map(Self)
            .ok_or_else(|| LaunchIdError(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A tab started for an agent before the agent's host session exists. The session's registration binds
/// to it and inherits where it runs, its title, parent and work item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub id: LaunchId,
    pub host: Host,
    pub title: Title,
    pub cwd: PathBuf,
    pub worktree: Option<PathBuf>,
    pub terminal: Option<Terminal>,
    pub wrapper: Wrapper,
    pub item: Option<WorkItemRef>,
    pub parent: Option<Handle>,
    pub created_at: DateTime<Utc>,
    /// The session most recently bound to this launch.
    pub session: Option<Handle>,
    /// The wrapper's lifecycle: a wrapper stops its agent when its launch or the bound session is closing.
    pub state: State,
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("launch {} has already ended", .0.as_str())]
pub struct LaunchEnded(pub LaunchId);

impl Launch {
    pub fn request_close(&mut self, now: DateTime<Utc>) -> Result<(), LaunchEnded> {
        if self.state.request_close(now) {
            Ok(())
        } else {
            Err(LaunchEnded(self.id.clone()))
        }
    }

    pub fn end(&mut self, reason: EndReason, now: DateTime<Utc>) -> Result<(), LaunchEnded> {
        if self.state.end(reason, now) {
            Ok(())
        } else {
            Err(LaunchEnded(self.id.clone()))
        }
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

impl State {
    /// Moves to closing unless already closing; false when ended.
    fn request_close(&mut self, now: DateTime<Utc>) -> bool {
        match self {
            Self::Active => {
                *self = Self::Closing { requested_at: now };
                true
            }
            Self::Closing { .. } => true,
            Self::Ended { .. } => false,
        }
    }

    /// Moves to ended; false when already ended.
    fn end(&mut self, reason: EndReason, now: DateTime<Utc>) -> bool {
        if let Self::Ended { .. } = self {
            return false;
        }
        *self = Self::Ended { at: now, reason };
        true
    }

    pub fn is_closing(&self) -> bool {
        matches!(self, Self::Closing { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    pub handle: Handle,
    pub cwd: PathBuf,
    pub worktree: Option<PathBuf>,
    pub terminal: Option<Terminal>,
    pub wrapper: Option<Wrapper>,
    pub title: Option<Title>,
    pub parent: Option<Handle>,
    pub launch: Option<LaunchId>,
    pub started_at: DateTime<Utc>,
}

impl Registration {
    /// Fills what the registration leaves unset from the launch it binds to.
    #[must_use]
    pub fn inherit(self, launch: &Launch) -> Self {
        Self {
            worktree: self.worktree.or_else(|| launch.worktree.clone()),
            terminal: self.terminal.or_else(|| launch.terminal.clone()),
            wrapper: self.wrapper.or(Some(launch.wrapper)),
            title: self.title.or_else(|| Some(launch.title.clone())),
            parent: self.parent.or_else(|| launch.parent.clone()),
            launch: Some(launch.id.clone()),
            ..self
        }
    }
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
    title: Option<Title>,
    parent: Option<Handle>,
    launch: Option<LaunchId>,
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
            title: registration.title,
            parent: registration.parent,
            launch: registration.launch,
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

    /// A resumed session re-registers: its location and process change and its links stay, as do its
    /// title, parent and launch unless the registration names new ones. An ended session becomes active
    /// again; a pending close request survives.
    pub fn resume(&mut self, registration: Registration) {
        self.cwd = registration.cwd;
        self.worktree = registration.worktree;
        self.terminal = registration.terminal;
        self.wrapper = registration.wrapper;
        if registration.title.is_some() {
            self.title = registration.title;
        }
        if registration.parent.is_some() {
            self.parent = registration.parent;
        }
        if registration.launch.is_some() {
            self.launch = registration.launch;
        }
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

    /// Links `item` with `role` unless the session already links it, keeping the existing role.
    pub fn link_if_absent(&mut self, item: WorkItemRef, role: Role, now: DateTime<Utc>) {
        if !self.links.iter().any(|link| link.item == item) {
            self.link(item, role, now);
        }
    }

    pub fn request_close(&mut self, now: DateTime<Utc>) -> Result<(), AlreadyEnded> {
        if self.state.request_close(now) {
            Ok(())
        } else {
            Err(AlreadyEnded(self.handle.clone()))
        }
    }

    pub fn end(&mut self, reason: EndReason, now: DateTime<Utc>) -> Result<(), AlreadyEnded> {
        if self.state.end(reason, now) {
            Ok(())
        } else {
            Err(AlreadyEnded(self.handle.clone()))
        }
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

    pub fn title(&self) -> Option<&Title> {
        self.title.as_ref()
    }

    pub fn parent(&self) -> Option<&Handle> {
        self.parent.as_ref()
    }

    pub fn launch(&self) -> Option<&LaunchId> {
        self.launch.as_ref()
    }

    pub fn started_at(&self) -> DateTime<Utc> {
        self.started_at
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn is_closing(&self) -> bool {
        self.state.is_closing()
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
            title: None,
            parent: None,
            launch: None,
            started_at: at(0),
        }
    }

    fn launch() -> Launch {
        Launch {
            id: LaunchId::parse("u-1").unwrap(),
            host: Host::parse("claude").unwrap(),
            title: Title::parse("core 461").unwrap(),
            cwd: "/w".into(),
            worktree: Some("/w.worktrees/F".into()),
            terminal: Some(Terminal::new("windows-terminal", Some("t")).unwrap()),
            wrapper: Wrapper {
                pid: 7,
                started_at: at(0),
            },
            item: Some(item("461")),
            parent: Some(Handle::parse("claude:parent").unwrap()),
            created_at: at(0),
            session: None,
            state: State::Active,
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
    fn inheriting_a_launch_fills_only_unset_fields() {
        let mut explicit = registration("/a");
        explicit.title = Some(Title::parse("mine").unwrap());
        let inherited = explicit.inherit(&launch());
        assert_eq!(inherited.title.unwrap().as_str(), "mine");
        assert_eq!(inherited.wrapper.unwrap().pid, 7);
        assert_eq!(inherited.parent.unwrap().to_string(), "claude:parent");
        assert_eq!(inherited.launch.unwrap().as_str(), "u-1");
        assert_eq!(inherited.worktree.unwrap(), PathBuf::from("/w.worktrees/F"));
        assert_eq!(inherited.cwd, PathBuf::from("/a"));
    }

    #[test]
    fn resume_keeps_title_and_parent_unless_replaced() {
        let mut session = Session::register(registration("/a").inherit(&launch()));
        session.resume(registration("/b"));
        assert_eq!(session.title().unwrap().as_str(), "core 461");
        assert_eq!(session.parent().unwrap().to_string(), "claude:parent");
        let mut renamed = registration("/b");
        renamed.title = Some(Title::parse("renamed").unwrap());
        session.resume(renamed);
        assert_eq!(session.title().unwrap().as_str(), "renamed");
    }

    #[test]
    fn linking_if_absent_keeps_an_existing_role() {
        let mut session = Session::register(registration("/a"));
        session.link(item("1"), Role::parse("reviewer").unwrap(), at(1));
        session.link_if_absent(item("1"), Role::parse("owner").unwrap(), at(2));
        session.link_if_absent(item("2"), Role::parse("owner").unwrap(), at(2));
        assert_eq!(session.links()[0].role.as_str(), "reviewer");
        assert_eq!(session.links()[1].role.as_str(), "owner");
    }

    #[test]
    fn titles_need_visible_text_and_launch_ids_no_whitespace() {
        assert!(Title::parse("  ").is_err());
        assert_eq!(Title::parse("core 461").unwrap().as_str(), "core 461");
        assert!(LaunchId::parse("a b").is_err());
        assert!(LaunchId::parse("").is_err());
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
