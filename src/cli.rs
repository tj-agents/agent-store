use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chrono::{DateTime, Utc};
use clap::error::ErrorKind;
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::domain::reference::{self, RepositoryScope, WorkItemRef};
use crate::domain::session::ports::{Item, StoreError};
use crate::domain::session::service::{self, EndError, LinkError, ShowError};
use crate::domain::session::{
    EndReason, Handle, Registration, Role, Session, State, Terminal, Wrapper,
};
use crate::git;
use crate::sqlite::{Store, legacy};

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
#[serde(tag = "command", rename_all = "kebab-case")]
enum Command {
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
        #[arg(long)]
        session: String,
        #[arg(long, default_value = "closed")]
        #[serde(default = "closed")]
        reason: String,
    },
    /// Ask the wrapper of a session, or of every open session of a work item, to close it.
    MarkClosing {
        #[arg(long, required_unless_present = "session", conflicts_with = "session")]
        item: Option<String>,
        #[arg(long)]
        session: Option<String>,
    },
    /// List the sessions linked to a work item.
    Sessions {
        #[arg(long)]
        item: String,
    },
    /// List every work item that has linked sessions.
    Items,
    /// Show one session and its links.
    Show { session: String },
    /// One-time import of the retired Python store (`sessions.db`, schema 4).
    ImportLegacy {
        #[arg(long)]
        from: PathBuf,
    },
}

fn owner() -> String {
    Role::OWNER.to_owned()
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
        Command::Register {
            session,
            cwd,
            worktree,
            terminal_kind,
            terminal_id,
            wrapper_pid,
            wrapper_started_at,
            started_at,
        } => {
            let terminal = terminal_kind
                .map(|kind| Terminal::new(&kind, terminal_id.as_deref()))
                .transpose()
                .map_err(|e| Failure::new(Code::InvalidInput, &e))?;
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
                cwd,
                worktree,
                terminal,
                wrapper,
                started_at: started_at.unwrap_or(now),
            };
            let session = service::register(store, registration)
                .await
                .map_err(|e| storage(&e))?;
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
        Command::End { session, reason } => {
            let reason =
                EndReason::parse(&reason).map_err(|e| Failure::new(Code::InvalidInput, &e))?;
            let session = service::end(store, &handle(&session)?, reason, now)
                .await
                .map_err(end_failure)?;
            Ok(json!({ "session": SessionView::from(&session) }))
        }
        Command::MarkClosing { item, session } => {
            let sessions = match (item, session) {
                (Some(item), None) => {
                    let item = resolve_item(&item, environment)?;
                    service::mark_item_closing(store, &item, now)
                        .await
                        .map_err(|e| storage(&e))?
                }
                (None, Some(session)) => {
                    vec![
                        service::mark_session_closing(store, &handle(&session)?, now)
                            .await
                            .map_err(end_failure)?,
                    ]
                }
                _ => return Err(Failure::invalid("pass exactly one of item or session")),
            };
            Ok(json!({ "sessions": sessions.iter().map(SessionView::from).collect::<Vec<_>>() }))
        }
        Command::Sessions { item } => {
            let item = resolve_item(&item, environment)?;
            let sessions = service::sessions(store, &item)
                .await
                .map_err(|e| storage(&e))?;
            Ok(json!({
                "item": ReferenceView::from(&item),
                "sessions": sessions.iter().map(SessionView::from).collect::<Vec<_>>(),
            }))
        }
        Command::Items => {
            let items = service::items(store).await.map_err(|e| storage(&e))?;
            Ok(json!({ "items": items.iter().map(ItemView::from).collect::<Vec<_>>() }))
        }
        Command::Show { session } => {
            let session = service::show(store, &handle(&session)?)
                .await
                .map_err(|e| match e {
                    ShowError::NotRegistered(e) => Failure::new(Code::NotFound, &e),
                    ShowError::Store(e) => storage(&e),
                })?;
            Ok(json!({ "session": SessionView::from(&session) }))
        }
        Command::ImportLegacy { from } => {
            let summary = legacy::import(store, &from).await.map_err(|e| {
                let code = match e {
                    legacy::ImportError::SchemaVersion { .. }
                    | legacy::ImportError::InvalidRow(_) => Code::InvalidInput,
                    legacy::ImportError::Read { .. } | legacy::ImportError::Store(_) => {
                        Code::Storage
                    }
                };
                Failure::new(code, &e)
            })?;
            Ok(json!({
                "sessions": summary.sessions,
                "links": summary.links,
                "unlinked_references": summary.unlinked_references,
            }))
        }
    }
}

fn handle(text: &str) -> Result<Handle, Failure> {
    Handle::parse(text).map_err(|e| Failure::new(Code::InvalidInput, &e))
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
    started_at: DateTime<Utc>,
    state: &'static str,
    closing_requested_at: Option<DateTime<Utc>>,
    ended_at: Option<DateTime<Utc>>,
    end_reason: Option<String>,
    links: Vec<LinkView>,
}

impl From<&Session> for SessionView {
    fn from(session: &Session) -> Self {
        let (state, closing_requested_at, ended_at, end_reason) = match session.state() {
            State::Active => ("active", None, None, None),
            State::Closing { requested_at } => ("closing", Some(*requested_at), None, None),
            State::Ended { at, reason } => {
                ("ended", None, Some(*at), Some(reason.as_str().to_owned()))
            }
        };
        Self {
            session: session.handle().to_string(),
            host: session.handle().host().as_str().to_owned(),
            native_id: session.handle().native_id().to_owned(),
            cwd: session.cwd().to_string_lossy().into_owned(),
            worktree: session.worktree().map(|p| p.to_string_lossy().into_owned()),
            terminal: session.terminal().map(|t| TerminalView {
                kind: t.kind().to_owned(),
                id: t.id().map(str::to_owned),
            }),
            wrapper: session.wrapper().map(|w| WrapperView {
                pid: w.pid,
                started_at: w.started_at,
            }),
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
