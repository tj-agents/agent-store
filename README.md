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
- explicit worktree ownership, retaining every claimed path for a session and its optional launch.

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
| `https://github.com/acme/app/issues/461` (or `/pull/461`) | `github:github.com/acme/app:461` |
| `https://dev.azure.com/acme/Web/_workitems/edit/461` (or `acme/_workitems/edit/461` without a project) | `azure-devops:dev.azure.com/acme:461` |
| `https://acme.visualstudio.com/Web/_workitems/edit/461` | `azure-devops:dev.azure.com/acme:461` |
| `https://tracker.example/items/APP-12` | `url:https://tracker.example/items/APP-12` |
| `url:https://tracker.example/items/APP-12` | the same URL reference |
| `provider:scope:id`, for example `jira:acme.atlassian.net:APP-12` | as written |

A provider is any lowercase name. For `github` and `azure-devops` the scope must be
`github.com/<owner>/<repo>` or `dev.azure.com/<org>` (lowercased) and the id a positive integer, stored
without leading zeros. Other absolute URLs are validated and normalized
by the Rust `url` crate: its scope is the scheme and its native id is everything after the scheme's colon.
Paths, nondefault ports, query strings and fragments remain part of that URL's identity. For example:

```json
{"ref":"url:https://tracker.example:8443/items/APP-12?view=full#note",
 "provider":"url","scope":"https",
 "native_id":"//tracker.example:8443/items/APP-12?view=full#note"}
```

Other providers' explicit scopes and ids, including existing `url` provider values, are kept as written.
An explicit `provider:scope:id` takes precedence over an ambiguous opaque URL, splitting scope and id
at the last colon. A valid `url:<scheme>://...` instead splits at the scheme's colon to preserve the full
URL remainder. Raw opaque URLs such as `mailto:work@example.com` and `custom:ticket-42` are also accepted.
Explicit URL wrappers preserve their fields as written; raw URLs receive URL normalization.

The CLI rejects bare `461` and `#461`; pass a full reference or URL. A code repository's remote does not
select its issue tracker. Library callers may pass an explicit `RepositoryScope` to `WorkItemRef::parse`
to resolve bare ids, using `RepositoryScope::new(provider, scope)` for their selected tracker.

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
| 4 | `conflict` | the session has already ended, its launch binding differs, or another owner holds the worktree |
| 5 | `storage` | the database cannot be opened, migrated, read or written, or holds invalid data |

A session is addressed by its handle, `host:session-id`, for example `claude:0f3c2a…`.

A **launch** is a terminal tab started for an agent before the agent's host assigns a session id. The
process that owns the tab records it under an id of its own choosing and hands that id to the agent; the
agent's session then registers with `--launch-id U --require-unowned` and inherits the launch's title, parent,
terminal, wrapper and work item. The same launch can retry registration. A replacement launch uses
[guarded resume](#guarded-resume) to take over a stopped session. A wrapper stops its agent when its launch
or the bound session is `closing`. A launch's `open`
records whether its tab is open, as its wrapper last reported; it starts `false`.

An active owner keeps its launch: another native session receives a conflict without changing either
session or the launch. An active launch can bind a replacement after its reciprocal owner has ended;
the old session's history remains unchanged. These checks and concurrent first bindings are atomic.

| Command | Result `data` |
|---|---|
| `launch --launch-id U --host H --title T --cwd DIR --wrapper-pid PID --wrapper-started-at TIME [--worktree DIR] [--item REF] [--terminal-kind K [--terminal-id ID]] [--parent H]` | `{launch}`. A launch id is recorded once. |
| `register --session H --cwd DIR [--worktree DIR] [--terminal-kind K [--terminal-id ID]] [--wrapper-pid PID --wrapper-started-at TIME] [--title T] [--parent H] [--launch-id U [--require-unowned]] [--started-at TIME]` | `{session}`. Relative paths resolve against the current directory. Re-registering updates where the session runs and preserves links and pending close requests. With `--launch-id`, the session inherits unspecified fields and links the launch's item as `owner` unless already linked. With `--require-unowned`, a different owner causes a conflict. |
| `resume --session H --from-launch-id OLD --launch-id NEW --from-wrapper-pid OLD_PID --from-wrapper-started-at OLD_TIME --wrapper-pid NEW_PID --wrapper-started-at NEW_TIME` | `{session}`. Compare the recorded identities and move the session, claims and open marker in one transaction. See [guarded resume](#guarded-resume). |
| `link --session H --item REF [--role owner]` | `{session}`. Linking again changes only the role. |
| `end --session H [--reason closed]` or `end --launch-id U [--reason closed]` | `{session}` or `{launch}` |
| `mark-closing --item REF`, `--session H` or `--launch-id U` | `{sessions, launches}`: those now `closing`. `--item` reaches every session linked to the item and every launch for it. |
| `sessions --item REF` or `sessions --title T` | `{item, sessions}` or `{title, sessions}`, oldest first |
| `items` | `{items: [{item, sessions: [{session, role}]}]}` for every item with a linked session |
| `set-open --launch-id U --open true\|false` | `{launch}`, whatever its state |
| `launches [--item REF]` | `{launches}`, oldest first |
| `show H` or `show --launch-id U` | `{session}`, or `{launch, session}` where `session` is `null` until one registers |
| `claim-worktree --session H [--launch-id U] --path DIR` | `{worktree}`. Claim an existing canonical directory for an active session and its exact active launch binding. |
| `worktrees --session H [--launch-id U]` | `{worktrees}`, sorted by path, for that exact session/launch pair. Omitting the launch selects unmanaged ownership only. |
| `release-worktree --session H [--launch-id U] --path PATH` | `{released: true}`. Release only that owner's claim; repeating an absent release succeeds. |

Times are RFC 3339, for example `2026-11-08T10:00:00Z`. A session object:

```json
{
  "session": "claude:abc", "host": "claude", "native_id": "abc",
  "cwd": "/src/app", "worktree": "/src/app.worktrees/Feature-X",
  "terminal": {"kind": "windows-terminal", "id": "tab-7"},
  "wrapper": {"pid": 4242, "started_at": "2026-11-08T10:00:00Z"},
  "title": "app 461", "parent": "claude:0f3c", "launch_id": "7d2e…",
  "started_at": "2026-11-08T10:00:01Z",
  "state": "active", "closing_requested_at": null, "ended_at": null, "end_reason": null,
  "links": [{"item": {"ref": "github:github.com/acme/app:461", "provider": "github",
             "scope": "github.com/acme/app", "native_id": "461"},
             "role": "owner", "linked_at": "2026-11-08T10:00:02Z"}]
}
```

A launch object:

```json
{
  "launch_id": "7d2e…", "host": "claude", "title": "app 461",
  "cwd": "/src/app.worktrees/Feature-X", "worktree": "/src/app.worktrees/Feature-X",
  "terminal": {"kind": "windows-terminal", "id": "tab-7"},
  "wrapper": {"pid": 4242, "started_at": "2026-11-08T10:00:00Z"},
  "item": {"ref": "github:github.com/acme/app:461", "provider": "github",
           "scope": "github.com/acme/app", "native_id": "461"},
  "parent": "claude:0f3c", "created_at": "2026-11-08T10:00:00Z", "session": "claude:abc",
  "state": "active", "closing_requested_at": null, "ended_at": null, "end_reason": null,
  "open": true
}
```

Writes are atomic: each command reads, changes and saves inside one write transaction, so commands run
in parallel by several processes never undo each other.

### Worktree ownership

```sh
agent-store claim-worktree --session codex:abc --launch-id run-1 --path /src/app/.worktrees/Fix-Parser
agent-store worktrees --session codex:abc --launch-id run-1
agent-store release-worktree --session codex:abc --launch-id run-1 --path /src/app/.worktrees/Fix-Parser
```

Claim after creating the directory. The claim command resolves its canonical path and returns
`{path, session, launch_id, claimed_at}`. Keep that exact returned path for release, including on Windows;
release works after the directory has disappeared. Repeating a claim by the same owner preserves its
original timestamp. A competing owner cannot claim or release the path, even after the owning session ends.

A session may own several worktrees across repositories. Re-registering it or ending its session or launch
preserves those claims. The caller releases each claim only after its cleanup succeeds. The store records
ownership; callers still establish whether a Git worktree is clean, merged and safe to remove. Existing
`cwd` and `worktree` fields describe location and do not create ownership claims during an upgrade.

### Guarded resume

Use `resume` when a stopped native conversation continues in a new managed launch:

```sh
agent-store resume --session codex:abc --from-launch-id run-1 --launch-id run-2 \
  --from-wrapper-pid 4242 --from-wrapper-started-at 2026-11-08T10:00:00Z \
  --wrapper-pid 9000 --wrapper-started-at 2026-11-09T11:00:00Z
```

Before invoking it, the caller must prove that **both the old native session and its wrapper have
stopped**. Wrapper death alone does not prove native death. The store does not inspect processes, infer
liveness from timestamps or decide whether recovery is authorized.

The session must still be reciprocally bound to `OLD`, with the expected old wrapper on both the session
and launch. `NEW` must be a different, existing, active, unbound launch for the same host with the expected
new wrapper. Neither the source launch nor the session may be closing; an ended source or session is
allowed after the caller establishes eligibility. A changed identity or closing state returns `conflict`
without changing anything. Competing resumes to different destinations cannot both succeed.

One write transaction adopts the destination's cwd, worktree location, terminal, wrapper, title and parent
(including an absent parent), activates the session, links the destination's item as owner if not already
linked, binds the destination and moves **all and only** claims for `(H, OLD)` to `(H, NEW)`. Claim paths
and `claimed_at`, existing links and roles, the session's original start time and the source launch's
history remain unchanged. There is no release/reclaim interval and no filesystem access to claim paths;
even a missing directory's claim is retained. Other sessions' claims and this session's claims under
other launches or unmanaged ownership remain unchanged. Ordinary `register` still never transfers claims.

An exact retry also requires a durable admission receipt for that session, source and destination,
written in the same transaction. It succeeds only while the session and active destination remain
reciprocally bound with the expected new wrapper, the source identity still matches and the source has
no remaining claims for this session. Ordinary `register` cannot create this receipt, even when it
produces identical metadata and the source has no claims. It does not reactivate an ended destination
or repair a partially rebound session. A retry does not overwrite session metadata or existing links.
Receipts apply only to an uninterrupted active binding. Changing the session's launch or wrapper, or
closing or ending the session, invalidates its receipts in the same transaction. Returning to an earlier
binding through ordinary registration cannot restore them. Registration that retains the same active
launch and wrapper preserves the receipt, including updates to location, title, parent or links. A
subsequent guarded resume replaces the previous admission with its own receipt; other sessions' receipts
and all worktree claims are unaffected by invalidation.
Upgrading creates no receipts for existing bindings; those bindings cannot establish a prior admission.

`state` is `active`, `closing` or `ended`. New fields may be added within a schema version; a field is never
removed or changed in meaning without incrementing `schema_version`.

### JSON requests

Instead of flags, pass one request object on stdin (or from a file) with `--input`. Field names are the
flag names in `snake_case`; `command` names the subcommand; an unknown field is invalid input:

```sh
echo '{"command":"link","session":"codex:019a…","item":"https://tracker.example/items/APP-12","role":"reviewer"}' | agent-store --input -
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
        title: None,
        parent: None,
        launch: None,
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
`domain::worktree::ports::Store` supplies atomic ownership updates and listing. The corresponding
`service::claim` and `service::release` take an `Owner` and a validated `worktree::Path`. Library callers
canonicalize an existing directory before constructing a claim path, then retain the stored path for release.

`domain::session::service::resume(&store, &Resume, now)` exposes the same guarded operation to library
callers. `domain::session::Resume` carries `handle`, `source`, `destination`, `source_wrapper` and
`destination_wrapper`. Its `LinkStore::rebind` port reads the identities, source-claim presence and exact
admission receipt, then saves the admitted session, destination binding, receipt and claim changes in one
transaction. Launch identities and wrappers are immutable; receipts reference those existing records.
Native and wrapper liveness verification belongs to the caller of this API as well.

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
