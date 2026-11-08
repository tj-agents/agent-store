# agent-store

A small SQLite store that links tracker work items to the AI coding-agent sessions working on them, with a
JSON command-line interface any language can call and a Rust library.

Use it to answer "which agents are working on issue 461, and where are they running?" after a crash, a
reboot or a closed terminal, and to ask those agents' wrappers to shut down when the item is done.

## What it stores, and what it does not

It stores **links only**:

- a **work item reference**: `provider:scope:id`, for example `github:github.com/acme/app:461`;
- a **session**: the host that runs it (`claude`, `codex`, or any other name), the host's own session id,
  its working directory and worktree, the terminal tab or pane it runs in, the pid and start time of the
  process that owns that tab, and when it started, was asked to close, and ended;
- which sessions are linked to which items, with an advisory role such as `owner` or `reviewer`.

It does **not** store:

- work item status, type or title: ask the tracker;
- whether a session's process is alive: ask the host (for example `claude agents --json`);
- conversation or turn state.

## Install

Download the archive for your platform from the
[releases](https://github.com/tj-agents/agent-store/releases), or build from source:

```sh
cargo install --git https://github.com/tj-agents/agent-store --locked
```

Release assets are static binaries with no runtime dependencies:
`agent-store-v<version>-x86_64-pc-windows-msvc.zip` and
`agent-store-v<version>-x86_64-unknown-linux-musl.tar.gz`, each with a `.sha256`.

## Database location

In order of precedence: `--database <file>`, the `AGENT_STORE_DATABASE` environment variable, then
`agent-store/agent-store.db` under the platform's local data directory (`%LOCALAPPDATA%` on Windows,
`$XDG_DATA_HOME` or `~/.local/share` on Linux, `~/Library/Application Support` on macOS). Rows hold
machine-specific pids, terminals and paths, so keep the file out of roaming or synced folders. The file and its
directory are created on first use and the schema is migrated automatically.

## Work item references

Anywhere a command takes `--item`, it accepts:

| Input | Resolves to |
|---|---|
| `461` or `#461` | the current directory's `origin` remote: GitHub or Azure DevOps |
| `https://github.com/acme/app/issues/461` (or `/pull/461`) | `github:github.com/acme/app:461` |
| `https://dev.azure.com/acme/Web/_workitems/edit/461` | `azure-devops:dev.azure.com/acme:461` |
| `https://acme.visualstudio.com/Web/_workitems/edit/461` | `azure-devops:dev.azure.com/acme:461` |
| `provider:scope:id`, for example `jira:acme.atlassian.net:APP-12` | as written |

A provider is any lowercase name. For `github` and `azure-devops` the scope must be
`github.com/<owner>/<repo>` or `dev.azure.com/<org>` (lowercased) and the id a positive integer, stored
without leading zeros; other providers' scopes and ids are kept as written.

## CLI

Every command prints exactly one JSON document to stdout (`--help` and `--version` print text):

```json
{"schema_version": 1, "ok": true, "data": { ... }}
{"schema_version": 1, "ok": false, "error": {"code": "not-found", "message": "session claude:x is not registered"}}
```

| Exit | `error.code` | Meaning |
|---|---|---|
| 0 | | success |
| 1 | `unexpected` | an internal failure |
| 2 | `invalid-input` | bad arguments, reference, handle or JSON request |
| 3 | `not-found` | the session is not registered |
| 4 | `conflict` | the session has already ended |
| 5 | `storage` | the database cannot be opened, migrated, read or written, or holds invalid data |

A session is addressed by its handle, `host:session-id`, for example `claude:0f3c2a…`.

| Command | Result `data` |
|---|---|
| `register --session H --cwd DIR [--worktree DIR] [--terminal-kind K [--terminal-id ID]] [--wrapper-pid PID --wrapper-started-at TIME] [--started-at TIME]` | `{session}`. Relative paths resolve against the current directory. Re-registering a session (a resume) updates where it runs and keeps its links; an ended session becomes active again, and a pending close request stays. |
| `link --session H --item REF [--role owner]` | `{session}`. Linking again changes only the role. |
| `end --session H [--reason closed]` | `{session}` |
| `mark-closing --item REF` or `mark-closing --session H` | `{sessions}`: the sessions now `closing`. A wrapper that polls `show` stops its agent when it sees this. |
| `sessions --item REF` | `{item, sessions}`, oldest first |
| `items` | `{items: [{item, sessions: [{session, role}]}]}` for every item with a linked session |
| `show H` | `{session}` |

Times are RFC 3339, for example `2026-11-08T10:00:00Z`. A session object:

```json
{
  "session": "claude:abc", "host": "claude", "native_id": "abc",
  "cwd": "/src/app", "worktree": "/src/app.worktrees/Feature-X",
  "terminal": {"kind": "windows-terminal", "id": "tab-7"},
  "wrapper": {"pid": 4242, "started_at": "2026-11-08T10:00:00Z"},
  "started_at": "2026-11-08T10:00:01Z",
  "state": "active", "closing_requested_at": null, "ended_at": null, "end_reason": null,
  "links": [{"item": {"ref": "github:github.com/acme/app:461", "provider": "github",
             "scope": "github.com/acme/app", "native_id": "461"},
             "role": "owner", "linked_at": "2026-11-08T10:00:02Z"}]
}
```

Writes are atomic: each command reads, changes and saves inside one write transaction, so commands run
in parallel by several processes never undo each other.

`state` is `active`, `closing` or `ended`. New fields may be added within a schema version; a field is never
removed or changed in meaning without incrementing `schema_version`.

### JSON requests

Instead of flags, pass one request object on stdin (or from a file) with `--input`. Field names are the
flag names in `snake_case`; `command` names the subcommand; an unknown field is invalid input:

```sh
echo '{"command":"link","session":"codex:019a…","item":"#461","role":"reviewer"}' | agent-store --input -
```

## Library

```rust
use std::path::Path;

use agent_store::domain::reference::WorkItemRef;
use agent_store::domain::session::{Handle, Registration, Role, service};
use agent_store::sqlite::Store;

async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let store = Store::open(Path::new("links.db")).await?;
    let handle = Handle::parse("claude:abc")?;
    let registration = Registration {
        handle: handle.clone(),
        cwd: "/src/app".into(),
        worktree: None,
        terminal: None,
        wrapper: None,
        started_at: chrono::Utc::now(),
    };
    service::register(&store, registration).await?;
    let item = WorkItemRef::parse("github:github.com/acme/app:461", None)?;
    let owner = Role::parse(Role::OWNER)?;
    service::link(&store, &handle, item.clone(), owner, chrono::Utc::now()).await?;
    for session in service::sessions(&store, &item).await? {
        println!("{}", session.handle());
    }
    Ok(())
}
```

`domain::session::ports::LinkStore` is the storage port; `sqlite::Store` implements it with SeaORM.

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

A release is published when a commit on `main` passes CI and its `Cargo.toml` version has no release yet:
bump `version` to release.

The `.agents/`, `.claude/`, `.codex/` and `AGENTS.md` files configure the maintainers' coding agents and are
not needed to build or use the store.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your
option. Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
this work, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional
terms or conditions.
