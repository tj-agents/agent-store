use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chrono::{DateTime, Utc};
use clap::error::ErrorKind;
use clap::{ArgGroup, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::domain::reference::{self, RepositoryScope, WorkItemRef};
use crate::domain::session::ports::{Item, StoreError};
use crate::domain::session::service::{
    self, EndError, LaunchError, LaunchStateError, LinkError, RegisterError, ShowError,
    ShowLaunchError,
};
use crate::domain::session::{
    EndReason, Handle, Host, Launch, LaunchId, Registration, Role, Session, State, Terminal, Title,
    Wrapper,
};
use crate::domain::worktree::ports::Store as _;
use crate::domain::worktree::{self, Owner, Ownership, OwnershipError};
use crate::git;
use crate::sqlite::Store;

pub const SCHEMA_VERSION: u32 = 1;
pub const DATABASE_ENV: &str = "AGENT_STORE_DATABASE";

/// Links tracker work items to agent host sessions. Every command prints one JSON document.
#[derive(Debug, Parser)]
#[command(name = "agent-store", version)]
struct Cli {
    /// Database file. Defaults to `$AGENT_STORE_DATABASE`, then `<data dir>/agent-store/agent-store.db`.
    #[arg(long, global = true)]
    database: Option<PathBuf>,
    /// Read the request as a JSON object with a `command` field from a file, or `-` for stdin.
    #[arg(long)]
    input: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case", deny_unknown_fields)]
enum Command {
    ClaimWorktree {
        #[arg(long)]
        session: String,
        #[arg(long)]
        launch_id: Option<String>,
        #[arg(long)]
        path: PathBuf,
    },
    ReleaseWorktree {
        #[arg(long)]
        session: String,
        #[arg(long)]
        launch_id: Option<String>,
        #[arg(long)]
        path: PathBuf,
    },
    Worktrees {
        #[arg(long)]
        session: String,
        #[arg(long)]
        launch_id: Option<String>,
    },
    /// Record a tab launched for an agent before the agent's host session exists.
    Launch {
        /// An id the launcher chose, passed on to the session's `register --launch-id`.
        #[arg(long)]
        launch_id: String,
        /// The host that will run the session, for example `claude`.
        #[arg(long)]
        host: String,
        /// The tab title, by which the session is addressed.
        #[arg(long)]
        title: String,
        #[arg(long)]
        cwd: PathBuf,
        #[arg(long)]
        worktree: Option<PathBuf>,
        /// The work item the session is linked to, as owner, when it registers.
        #[arg(long)]
        item: Option<String>,
        #[arg(long)]
        terminal_kind: Option<String>,
        #[arg(long, requires = "terminal_kind")]
        terminal_id: Option<String>,
        /// Pid of the process that owns the tab.
        #[arg(long)]
        wrapper_pid: u32,
        #[arg(long)]
        wrapper_started_at: DateTime<Utc>,
        /// The session that started this one, as `host:session-id`.
        #[arg(long)]
        parent: Option<String>,
    },
    /// Record a session, or re-register a resumed one.
    Register {
        /// `host:session-id`, for example `claude:0f3c…`.
        #[arg(long)]
        session: String,
        #[arg(long)]
        cwd: PathBuf,
        #[arg(long)]
        worktree: Option<PathBuf>,
        /// Terminal kind, for example `windows-terminal` or `tmux`.
        #[arg(long)]
        terminal_kind: Option<String>,
        /// Tab or pane id within the terminal.
        #[arg(long, requires = "terminal_kind")]
        terminal_id: Option<String>,
        /// Pid of the process that owns the session's tab.
        #[arg(long, requires = "wrapper_started_at")]
        wrapper_pid: Option<u32>,
        #[arg(long, requires = "wrapper_pid")]
        wrapper_started_at: Option<DateTime<Utc>>,
        /// Bind to this launch, inheriting what these arguments leave unset and linking its work item.
        #[arg(long)]
        launch_id: Option<String>,
        #[arg(long)]
        title: Option<String>,
        /// The session that started this one, as `host:session-id`.
        #[arg(long)]
        parent: Option<String>,
        /// Defaults to now.
        #[arg(long)]
        started_at: Option<DateTime<Utc>>,
    },
    /// Link a session to a work item.
    Link {
        #[arg(long)]
        session: String,
        /// `461`, `#461`, a GitHub or Azure DevOps URL, or `provider:scope:id`.
        #[arg(long)]
        item: String,
        #[arg(long, default_value = Role::OWNER)]
        #[serde(default = "owner")]
        role: String,
    },
    /// Record that a session ended.
    End {
        #[arg(
            long,
            required_unless_present = "launch_id",
            conflicts_with = "launch_id"
        )]
        session: Option<String>,
        /// Record that a launch's wrapper finished, instead of a session.
        #[arg(long)]
        launch_id: Option<String>,
        #[arg(long, default_value = "closed")]
        #[serde(default = "closed")]
        reason: String,
    },
    /// Ask the wrapper of a session or launch, or of every open session and launch of a work item, to
    /// close it.
    #[command(group(ArgGroup::new("target").required(true).args(["item", "session", "launch_id"])))]
    MarkClosing {
        #[arg(long)]
        item: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        launch_id: Option<String>,
    },
    /// List every launch, or every launch for a work item, oldest first.
    Launches {
        #[arg(long)]
        item: Option<String>,
    },
    /// List every session, the sessions linked to a work item, or those titled `--title`.
    Sessions {
        #[arg(long, conflicts_with = "title")]
        item: Option<String>,
        #[arg(long)]
        title: Option<String>,
    },
    /// List every work item that has linked sessions.
    Items,
    /// Show one session and its links, or a launch and the session bound to it.
    Show {
        #[arg(required_unless_present = "launch_id", conflicts_with = "launch_id")]
        session: Option<String>,
        #[arg(long)]
        launch_id: Option<String>,
    },
}

fn owner() -> String {
    Role::OWNER.to_owned()
}

fn worktree_owner(session: &str, launch: Option<&str>) -> Result<Owner, Failure> {
    Ok(Owner {
        session: handle(session)?,
        launch: launch.map(parse_launch_id).transpose()?,
    })
}

fn ownership_view(owned: &Ownership) -> Value {
    json!({
        "path": owned.path.as_str(),
        "session": owned.owner.session.to_string(),
        "launch_id": owned.owner.launch.as_ref().map(LaunchId::as_str),
        "claimed_at": owned.claimed_at,
    })
}

fn ownership_failure(error: &OwnershipError) -> Failure {
    let code = match error {
        OwnershipError::NotRegistered(_) => Code::NotFound,
        OwnershipError::Binding | OwnershipError::Claimed(_) => Code::Conflict,
        OwnershipError::InvalidPath => Code::InvalidInput,
        OwnershipError::Store(_) => Code::Storage,
    };
    Failure::new(code, error)
}

fn closed() -> String {
    "closed".to_owned()
}

/// What the process reads from its environment, supplied by `main`.
pub struct Environment {
    pub database: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy)]
enum Code {
    Unexpected = 1,
    InvalidInput = 2,
    NotFound = 3,
    Conflict = 4,
    Storage = 5,
}

impl Code {
    fn name(self) -> &'static str {
        match self {
            Self::Unexpected => "unexpected",
            Self::InvalidInput => "invalid-input",
            Self::NotFound => "not-found",
            Self::Conflict => "conflict",
            Self::Storage => "storage",
        }
    }
}

struct Failure {
    code: Code,
    message: String,
}

impl Failure {
    fn new(code: Code, error: &(dyn std::error::Error + 'static)) -> Self {
        let mut message = error.to_string();
        let mut source = error.source();
        while let Some(cause) = source {
            message.push_str(": ");
            message.push_str(&cause.to_string());
            source = cause.source();
        }
        Self { code, message }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: Code::InvalidInput,
            message: message.into(),
        }
    }
}

/// Runs one command and writes its JSON result to `out`.
pub fn run(
    args: impl IntoIterator<Item = OsString>,
    environment: &Environment,
    stdin: &mut impl Read,
    out: &mut impl Write,
) -> ExitCode {
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => match error.kind() {
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                let _ = write!(out, "{}", error.render());
                return ExitCode::SUCCESS;
            }
            _ => {
                let message = error.render().to_string();
                return report(out, Err(Failure::invalid(message.trim())));
            }
        },
    };
    report(out, execute(cli, environment, stdin))
}

fn execute(cli: Cli, environment: &Environment, stdin: &mut impl Read) -> Result<Value, Failure> {
    let command = match (cli.command, cli.input) {
        (Some(command), None) => command,
        (None, Some(input)) => read_request(&input, stdin)?,
        (Some(_), Some(_)) => return Err(Failure::invalid("pass a command or --input, not both")),
        (None, None) => return Err(Failure::invalid("a command or --input is required")),
    };
    let database = cli
        .database
        .or_else(|| environment.database.clone())
        .or_else(|| {
            environment
                .data_dir
                .as_ref()
                .map(|dir| dir.join("agent-store").join("agent-store.db"))
        })
        .ok_or_else(|| {
            Failure::invalid(format!(
                "no data directory: pass --database or set {DATABASE_ENV}"
            ))
        })?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Failure::new(Code::Unexpected, &e))?;
    runtime.block_on(async {
        let store = Store::open(&database)
            .await
            .map_err(|e| Failure::new(Code::Storage, &e))?;
        dispatch(command, &store, environment).await
    })
}

fn read_request(input: &Path, stdin: &mut impl Read) -> Result<Command, Failure> {
    let mut text = String::new();
    if input == Path::new("-") {
        stdin.read_to_string(&mut text)
    } else {
        std::fs::File::open(input).and_then(|mut file| file.read_to_string(&mut text))
    }
    .map_err(|e| Failure::new(Code::InvalidInput, &e))?;
    serde_json::from_str(&text).map_err(|e| Failure::new(Code::InvalidInput, &e))
}

async fn dispatch(
    command: Command,
    store: &Store,
    environment: &Environment,
) -> Result<Value, Failure> {
    let now = environment.now;
    match command {
        Command::ClaimWorktree {
            session,
            launch_id,
            path,
        } => {
            let path = std::fs::canonicalize(absolute(path, environment)?)
                .map_err(|error| Failure::new(Code::InvalidInput, &error))?;
            if !path.is_dir() {
                return Err(Failure::invalid("worktree path must be a directory"));
            }
            let path = worktree::Path::new(&path).map_err(|error| ownership_failure(&error))?;
            let owned = worktree::service::claim(
                store,
                &worktree_owner(&session, launch_id.as_deref())?,
                &path,
                now,
            )
            .await
            .map_err(|error| ownership_failure(&error))?;
            Ok(json!({"worktree": ownership_view(&owned)}))
        }
        Command::ReleaseWorktree {
            session,
            launch_id,
            path,
        } => {
            let path = worktree::Path::new(&path).map_err(|error| ownership_failure(&error))?;
            worktree::service::release(
                store,
                &worktree_owner(&session, launch_id.as_deref())?,
                &path,
            )
            .await
            .map_err(|error| ownership_failure(&error))?;
            Ok(json!({"released": true}))
        }
        Command::Worktrees { session, launch_id } => {
            let owned = store
                .worktrees(&worktree_owner(&session, launch_id.as_deref())?)
                .await
                .map_err(|error| storage(&error))?;
            Ok(json!({"worktrees": owned.iter().map(ownership_view).collect::<Vec<_>>()}))
        }
        Command::Launch {
            launch_id,
            host,
            title,
            cwd,
            worktree,
            item,
            terminal_kind,
            terminal_id,
            wrapper_pid,
            wrapper_started_at,
            parent,
        } => {
            let launch = Launch {
                id: parse_launch_id(&launch_id)?,
                host: Host::parse(&host).map_err(|e| Failure::new(Code::InvalidInput, &e))?,
                title: parse_title(&title)?,
                cwd: absolute(cwd, environment)?,
                worktree: worktree.map(|w| absolute(w, environment)).transpose()?,
                terminal: terminal(terminal_kind, terminal_id)?,
                wrapper: Wrapper {
                    pid: wrapper_pid,
                    started_at: wrapper_started_at,
                },
                item: item.map(|i| resolve_item(&i, environment)).transpose()?,
                parent: parent.as_deref().map(handle).transpose()?,
                created_at: now,
                session: None,
                state: State::Active,
            };
            let launch = service::launch(store, launch).await.map_err(|e| match e {
                LaunchError::Exists(e) => Failure::new(Code::Conflict, &e),
                LaunchError::Store(e) => storage(&e),
            })?;
            Ok(json!({ "launch": LaunchView::from(&launch) }))
        }
        Command::Register {
            session,
            cwd,
            worktree,
            terminal_kind,
            terminal_id,
            wrapper_pid,
            wrapper_started_at,
            launch_id,
            title,
            parent,
            started_at,
        } => {
            let wrapper = match (wrapper_pid, wrapper_started_at) {
                (Some(pid), Some(started_at)) => Some(Wrapper { pid, started_at }),
                (None, None) => None,
                _ => {
                    return Err(Failure::invalid(
                        "wrapper_pid and wrapper_started_at go together",
                    ));
                }
            };
            let registration = Registration {
                handle: handle(&session)?,
                cwd: absolute(cwd, environment)?,
                worktree: worktree.map(|w| absolute(w, environment)).transpose()?,
                terminal: terminal(terminal_kind, terminal_id)?,
                wrapper,
                title: title.as_deref().map(parse_title).transpose()?,
                parent: parent.as_deref().map(handle).transpose()?,
                launch: None,
                started_at: started_at.unwrap_or(now),
            };
            let session = match launch_id {
                Some(launch_id) => service::register_launched(
                    store,
                    registration,
                    &parse_launch_id(&launch_id)?,
                    now,
                )
                .await
                .map_err(|e| match e {
                    RegisterError::UnknownLaunch(e) => Failure::new(Code::NotFound, &e),
                    RegisterError::HostMismatch { .. } => Failure::new(Code::InvalidInput, &e),
                    RegisterError::Store(e) => storage(&e),
                })?,
                None => service::register(store, registration)
                    .await
                    .map_err(|e| storage(&e))?,
            };
            Ok(json!({ "session": SessionView::from(&session) }))
        }
        Command::Link {
            session,
            item,
            role,
        } => {
            let item = resolve_item(&item, environment)?;
            let role = Role::parse(&role).map_err(|e| Failure::new(Code::InvalidInput, &e))?;
            let session = service::link(store, &handle(&session)?, item, role, now)
                .await
                .map_err(|e| match e {
                    LinkError::NotRegistered(e) => Failure::new(Code::NotFound, &e),
                    LinkError::Store(e) => storage(&e),
                })?;
            Ok(json!({ "session": SessionView::from(&session) }))
        }
        Command::End {
            session,
            launch_id,
            reason,
        } => {
            let reason =
                EndReason::parse(&reason).map_err(|e| Failure::new(Code::InvalidInput, &e))?;
            match (session, launch_id) {
                (Some(session), None) => {
                    let session = service::end(store, &handle(&session)?, reason, now)
                        .await
                        .map_err(end_failure)?;
                    Ok(json!({ "session": SessionView::from(&session) }))
                }
                (None, Some(launch_id)) => {
                    let launch =
                        service::end_launch(store, &parse_launch_id(&launch_id)?, reason, now)
                            .await
                            .map_err(launch_state_failure)?;
                    Ok(json!({ "launch": LaunchView::from(&launch) }))
                }
                _ => Err(Failure::invalid("pass exactly one of session or launch_id")),
            }
        }
        Command::Launches { item } => {
            let item = item.map(|i| resolve_item(&i, environment)).transpose()?;
            let launches = service::launches(store, item.as_ref())
                .await
                .map_err(|e| storage(&e))?;
            Ok(json!({ "launches": launches.iter().map(LaunchView::from).collect::<Vec<_>>() }))
        }
        Command::MarkClosing {
            item,
            session,
            launch_id,
        } => {
            let (sessions, launches) = match (item, session, launch_id) {
                (Some(item), None, None) => {
                    let item = resolve_item(&item, environment)?;
                    service::mark_item_closing(store, &item, now)
                        .await
                        .map_err(|e| storage(&e))?
                }
                (None, Some(session), None) => {
                    let session = service::mark_session_closing(store, &handle(&session)?, now)
                        .await
                        .map_err(end_failure)?;
                    (vec![session], Vec::new())
                }
                (None, None, Some(launch_id)) => {
                    let launch =
                        service::mark_launch_closing(store, &parse_launch_id(&launch_id)?, now)
                            .await
                            .map_err(launch_state_failure)?;
                    (Vec::new(), vec![launch])
                }
                _ => {
                    return Err(Failure::invalid(
                        "pass exactly one of item, session or launch_id",
                    ));
                }
            };
            Ok(json!({
                "sessions": sessions.iter().map(SessionView::from).collect::<Vec<_>>(),
                "launches": launches.iter().map(LaunchView::from).collect::<Vec<_>>(),
            }))
        }
        Command::Sessions { item, title } => match (item, title) {
            (Some(item), None) => {
                let item = resolve_item(&item, environment)?;
                let sessions = service::sessions(store, &item)
                    .await
                    .map_err(|e| storage(&e))?;
                Ok(json!({
                    "item": ReferenceView::from(&item),
                    "sessions": sessions.iter().map(SessionView::from).collect::<Vec<_>>(),
                }))
            }
            (None, Some(title)) => {
                let title = parse_title(&title)?;
                let sessions = service::titled(store, &title)
                    .await
                    .map_err(|e| storage(&e))?;
                Ok(json!({
                    "title": title.as_str(),
                    "sessions": sessions.iter().map(SessionView::from).collect::<Vec<_>>(),
                }))
            }
            (None, None) => {
                let sessions = service::all_sessions(store)
                    .await
                    .map_err(|e| storage(&e))?;
                Ok(json!({
                    "sessions": sessions.iter().map(SessionView::from).collect::<Vec<_>>(),
                }))
            }
            (Some(_), Some(_)) => unreachable!("clap rejects --item with --title"),
        },
        Command::Items => {
            let items = service::items(store).await.map_err(|e| storage(&e))?;
            Ok(json!({ "items": items.iter().map(ItemView::from).collect::<Vec<_>>() }))
        }
        Command::Show { session, launch_id } => match (session, launch_id) {
            (Some(session), None) => {
                let session =
                    service::show(store, &handle(&session)?)
                        .await
                        .map_err(|e| match e {
                            ShowError::NotRegistered(e) => Failure::new(Code::NotFound, &e),
                            ShowError::Store(e) => storage(&e),
                        })?;
                Ok(json!({ "session": SessionView::from(&session) }))
            }
            (None, Some(launch_id)) => {
                let (launch, session) = service::show_launch(store, &parse_launch_id(&launch_id)?)
                    .await
                    .map_err(|e| match e {
                        ShowLaunchError::UnknownLaunch(e) => Failure::new(Code::NotFound, &e),
                        ShowLaunchError::Store(e) => storage(&e),
                    })?;
                Ok(json!({
                    "launch": LaunchView::from(&launch),
                    "session": session.as_ref().map(SessionView::from),
                }))
            }
            _ => Err(Failure::invalid("pass exactly one of session or launch_id")),
        },
    }
}

fn absolute(path: PathBuf, environment: &Environment) -> Result<PathBuf, Failure> {
    if path.has_root() {
        return Ok(path);
    }
    environment
        .cwd
        .as_ref()
        .map(|cwd| cwd.join(&path))
        .ok_or_else(|| Failure::invalid(format!("cannot resolve relative path {}", path.display())))
}

fn handle(text: &str) -> Result<Handle, Failure> {
    Handle::parse(text).map_err(|e| Failure::new(Code::InvalidInput, &e))
}

fn parse_title(text: &str) -> Result<Title, Failure> {
    Title::parse(text).map_err(|e| Failure::new(Code::InvalidInput, &e))
}

fn parse_launch_id(text: &str) -> Result<LaunchId, Failure> {
    LaunchId::parse(text).map_err(|e| Failure::new(Code::InvalidInput, &e))
}

fn terminal(kind: Option<String>, id: Option<String>) -> Result<Option<Terminal>, Failure> {
    match (kind, id) {
        (Some(kind), id) => Terminal::new(&kind, id.as_deref())
            .map(Some)
            .map_err(|e| Failure::new(Code::InvalidInput, &e)),
        (None, None) => Ok(None),
        (None, Some(_)) => Err(Failure::invalid("terminal_id needs terminal_kind")),
    }
}

fn resolve_item(text: &str, environment: &Environment) -> Result<WorkItemRef, Failure> {
    let parsed = match WorkItemRef::parse(text, None) {
        Err(reference::ParseError::NeedsRepository(_)) => {
            let scope = environment
                .cwd
                .as_deref()
                .and_then(git::origin_url)
                .and_then(|url| RepositoryScope::from_remote_url(&url))
                .ok_or_else(|| {
                    Failure::invalid(format!(
                        "`{text}` is a bare id and the current directory has no GitHub or Azure DevOps `origin` remote; pass provider:scope:id or a URL"
                    ))
                })?;
            WorkItemRef::parse(text, Some(&scope))
        }
        parsed => parsed,
    };
    parsed.map_err(|e| Failure::new(Code::InvalidInput, &e))
}

fn launch_state_failure(error: LaunchStateError) -> Failure {
    match error {
        LaunchStateError::UnknownLaunch(e) => Failure::new(Code::NotFound, &e),
        LaunchStateError::Ended(e) => Failure::new(Code::Conflict, &e),
        LaunchStateError::Store(e) => storage(&e),
    }
}

fn end_failure(error: EndError) -> Failure {
    match error {
        EndError::NotRegistered(e) => Failure::new(Code::NotFound, &e),
        EndError::AlreadyEnded(e) => Failure::new(Code::Conflict, &e),
        EndError::Store(e) => storage(&e),
    }
}

fn storage(error: &StoreError) -> Failure {
    Failure::new(Code::Storage, error)
}

fn report(out: &mut impl Write, result: Result<Value, Failure>) -> ExitCode {
    let (document, code) = match result {
        Ok(data) => (
            json!({ "schema_version": SCHEMA_VERSION, "ok": true, "data": data }),
            0,
        ),
        Err(failure) => (
            json!({
                "schema_version": SCHEMA_VERSION,
                "ok": false,
                "error": { "code": failure.code.name(), "message": failure.message },
            }),
            failure.code as u8,
        ),
    };
    if writeln!(out, "{document}").is_err() {
        return ExitCode::from(Code::Unexpected as u8);
    }
    ExitCode::from(code)
}

#[derive(Serialize)]
struct ReferenceView {
    #[serde(rename = "ref")]
    canonical: String,
    provider: String,
    scope: String,
    native_id: String,
}

impl From<&WorkItemRef> for ReferenceView {
    fn from(item: &WorkItemRef) -> Self {
        Self {
            canonical: item.to_string(),
            provider: item.provider().as_str().to_owned(),
            scope: item.scope().as_str().to_owned(),
            native_id: item.native_id().as_str().to_owned(),
        }
    }
}

#[derive(Serialize)]
struct TerminalView {
    kind: String,
    id: Option<String>,
}

#[derive(Serialize)]
struct WrapperView {
    pid: u32,
    started_at: DateTime<Utc>,
}

impl From<&Terminal> for TerminalView {
    fn from(terminal: &Terminal) -> Self {
        Self {
            kind: terminal.kind().to_owned(),
            id: terminal.id().map(str::to_owned),
        }
    }
}

impl From<&Wrapper> for WrapperView {
    fn from(wrapper: &Wrapper) -> Self {
        Self {
            pid: wrapper.pid,
            started_at: wrapper.started_at,
        }
    }
}

#[derive(Serialize)]
struct LaunchView {
    launch_id: String,
    host: String,
    title: String,
    cwd: String,
    worktree: Option<String>,
    terminal: Option<TerminalView>,
    wrapper: WrapperView,
    item: Option<ReferenceView>,
    parent: Option<String>,
    created_at: DateTime<Utc>,
    session: Option<String>,
    state: &'static str,
    closing_requested_at: Option<DateTime<Utc>>,
    ended_at: Option<DateTime<Utc>>,
    end_reason: Option<String>,
}

impl From<&Launch> for LaunchView {
    fn from(launch: &Launch) -> Self {
        let (state, closing_requested_at, ended_at, end_reason) = state_fields(&launch.state);
        Self {
            launch_id: launch.id.as_str().to_owned(),
            host: launch.host.as_str().to_owned(),
            title: launch.title.as_str().to_owned(),
            cwd: launch.cwd.to_string_lossy().into_owned(),
            worktree: launch
                .worktree
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            terminal: launch.terminal.as_ref().map(TerminalView::from),
            wrapper: WrapperView::from(&launch.wrapper),
            item: launch.item.as_ref().map(ReferenceView::from),
            parent: launch.parent.as_ref().map(ToString::to_string),
            created_at: launch.created_at,
            session: launch.session.as_ref().map(ToString::to_string),
            state,
            closing_requested_at,
            ended_at,
            end_reason,
        }
    }
}

#[derive(Serialize)]
struct LinkView {
    item: ReferenceView,
    role: String,
    linked_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct SessionView {
    session: String,
    host: String,
    native_id: String,
    cwd: String,
    worktree: Option<String>,
    terminal: Option<TerminalView>,
    wrapper: Option<WrapperView>,
    title: Option<String>,
    parent: Option<String>,
    launch_id: Option<String>,
    started_at: DateTime<Utc>,
    state: &'static str,
    closing_requested_at: Option<DateTime<Utc>>,
    ended_at: Option<DateTime<Utc>>,
    end_reason: Option<String>,
    links: Vec<LinkView>,
}

impl From<&Session> for SessionView {
    fn from(session: &Session) -> Self {
        let (state, closing_requested_at, ended_at, end_reason) = state_fields(session.state());
        Self {
            session: session.handle().to_string(),
            host: session.handle().host().as_str().to_owned(),
            native_id: session.handle().native_id().to_owned(),
            cwd: session.cwd().to_string_lossy().into_owned(),
            worktree: session.worktree().map(|p| p.to_string_lossy().into_owned()),
            terminal: session.terminal().map(TerminalView::from),
            wrapper: session.wrapper().as_ref().map(WrapperView::from),
            title: session.title().map(|t| t.as_str().to_owned()),
            parent: session.parent().map(ToString::to_string),
            launch_id: session.launch().map(|l| l.as_str().to_owned()),
            started_at: session.started_at(),
            state,
            closing_requested_at,
            ended_at,
            end_reason,
            links: session
                .links()
                .iter()
                .map(|link| LinkView {
                    item: ReferenceView::from(&link.item),
                    role: link.role.as_str().to_owned(),
                    linked_at: link.linked_at,
                })
                .collect(),
        }
    }
}

type StateFields = (
    &'static str,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<String>,
);

fn state_fields(state: &State) -> StateFields {
    match state {
        State::Active => ("active", None, None, None),
        State::Closing { requested_at } => ("closing", Some(*requested_at), None, None),
        State::Ended { at, reason } => ("ended", None, Some(*at), Some(reason.as_str().to_owned())),
    }
}

#[derive(Serialize)]
struct ItemSessionView {
    session: String,
    role: String,
}

#[derive(Serialize)]
struct ItemView {
    item: ReferenceView,
    sessions: Vec<ItemSessionView>,
}

impl From<&Item> for ItemView {
    fn from(item: &Item) -> Self {
        Self {
            item: ReferenceView::from(&item.reference),
            sessions: item
                .sessions
                .iter()
                .map(|(handle, role)| ItemSessionView {
                    session: handle.to_string(),
                    role: role.as_str().to_owned(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn command_definition_is_consistent() {
        Cli::command().debug_assert();
    }
}
