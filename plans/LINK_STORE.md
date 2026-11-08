# Link store

Issue: [#2](https://github.com/tj-agents/agent-store/issues/2). Vision: TJ Agents phase 2, replacing the Python
store in `tj-agents/agent` (`.agents/agent-store/runtime/`).

## Decisions

- SQLite through SeaORM (`seaorm` standard, tj-agents/rust#32 and #33). SeaORM over Diesel for entities,
  ActiveModel and `sea-orm-migration` run at startup; Diesel is synchronous and DSL-first.
- Links only. No tracker status or kinds, no turn state, no liveness: callers read liveness from the hosts.
- Open-ended providers, hosts and roles (lowercase names); `github` and `azure-devops` get URL parsing and
  numeric-id validation. Role is advisory, with no single-owner constraint.
- Public, `MIT OR Apache-2.0`. tj-agents configuration stays in `.agents/`, `.claude/`, `.codex/` and
  `AGENTS.md`; the crate and README carry no tj-agents, machine or employer defaults.

## Schema (migration `m20261108_000001_create_link_store`)

- `work_item_references(id, provider, scope, native_id)`, unique `(provider, scope, native_id)`.
- `sessions(id, host, native_id, cwd, worktree_path, terminal_kind, terminal_id, wrapper_pid,
  wrapper_started_at, started_at, closing_requested_at, ended_at, end_reason)`, unique `(host, native_id)`.
- `session_links(session_id, reference_id, role, linked_at)`, primary key on the pair, cascading foreign keys.

## Interface

CLI commands `register`, `link`, `end`, `mark-closing`, `sessions`, `items`, `show`, and the temporary
`import-legacy`; flags or one JSON request via `--input`; one schema-versioned JSON document on stdout;
exit codes 0–5 (README). Every write is load-change-save inside one `BEGIN IMMEDIATE` transaction. Library: `domain` (references, sessions, `LinkStore` port, service use cases) and
`sqlite::Store`.

## Release

`release.yml` runs after CI succeeds on a push to `main`. When `Cargo.toml`'s version has no `v<version>`
release, it builds static `x86_64-pc-windows-msvc` (`+crt-static`) and `x86_64-unknown-linux-musl` binaries
through `build.yml`, which CI also runs on every PR, and publishes both archives with SHA-256 files. Consumers pin a release; Renovate's github-releases datasource bumps the pin.

## Verification

- `cargo fmt`, `clippy -D warnings`, unit tests and `tests/it` (real binary, real temporary SQLite: round
  trips, error codes, JSON input, bare-id resolution through a git remote, concurrent first use by eight
  processes, legacy import and schema-version refusal).
- Import dry run against a copy of the real `sessions.db`: 248 sessions, 16 links, 12 items, 3 references
  with no session.

## Follow-up

`CUTOVER.md`: switch `agent`'s callers to the CLI, import once, delete the Python store and `import-legacy`.
