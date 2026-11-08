# Cutover: agent onto the agent-store CLI

Issue: open one in `tj-agents/agent` when the session starts, linking
[agent-store#2](https://github.com/tj-agents/agent-store/issues/2). Run in its own session, in an `agent`
worktree, after no other session owns the files below. Personal clean-cut rule: one PR migrates every
caller, imports once and deletes the Python store and the import command.

## Preconditions

- `agent-store` has a published release (Windows and Linux archives).
- tj-agents/rust#31 merged and `rust-seaorm` is in the marketplace roster.

## Steps

1. **Distribution.** `agent` pins an `agent-store` release version in one file and installs the matching
   archive into its runtime bin directory from its existing install step (`install.ps1`, `install.sh`),
   verifying the `.sha256`. Add a Renovate custom manager (github-releases datasource,
   `tj-agents/agent-store`) for that file. Pass `--database ~/.agents-state/agent-store.db` (or set
   `AGENT_STORE_DATABASE` once in the shared runtime) so every caller uses one file.
2. **Callers.** Replace in-process `session_store` use with one helper that runs the CLI and parses its JSON
   (`schema_version` 1, exit codes 0–5):
   - `utility/peer/scripts/register_session.py` → `agent-store register` (host, session id, cwd, worktree,
     terminal kind/id, wrapper pid/start). Drop the `cli-sessions/*.json` registry and the ctypes process
     table where only they fed the store; liveness comes from `claude agents --json` and the Codex index.
   - `agent-store/runtime/session_hook.py`: SessionStart → `register`; SessionEnd → `end`; UserPromptSubmit
     and Stop no longer write (turn state is not stored). Remove those hook registrations.
   - `runtime/work_item_sessions.py` (`claim`, `bind`, `register`, `end`, `item`, `find`, `resume`,
     `interrupted`, `status`) → `link` / `end` / `sessions --item` / `items` / `show`. Ownership claiming
     and `execution:` items go: an item is its tracker reference. `status` reads the tracker.
   - `runtime/tj_agent/recovery.py` → `sessions --item <ref>` plus host liveness, for every agent of the item.
   - `utility/peer/scripts/session_close.py` and `client_close_watcher.py` → `end` / `mark-closing`.
     Decide in the PR whether anything still needs deleting sessions; the store has no delete today.
   - `utility/message/scripts/codex_followup.py` → `show`.
   - `agent-store:inspect` becomes a thin skill over `agent-store items|sessions|show` (trigger phrases in
     the description, the exact command in the body, `allowed-tools` on Claude, no logic in prose).
3. **Core.** Remove Core's path-load of the store (the `core#536` lockstep) in the same window; Core calls
   the CLI or nothing.
4. **Import once.** `agent-store --database ~/.agents-state/agent-store.db import-legacy --from
   ~/.agents-state/sessions.db` on each machine that has a `sessions.db`; record the summary in the PR.
5. **Delete.** The Python store (`.agents/agent-store/runtime/`), its loader and observer-staging lists,
   `tests/test_session_store.py` / `test_agent_store.py` and other store-only tests, then in `agent-store`
   remove `import-legacy` and `src/sqlite/legacy.rs` in a release after the import.
6. **Verify** on Windows and Linux from fresh Claude and Codex sessions: SessionStart registers, `link`
   from `work_item_sessions`, `tj reload <ref>` finds every agent, SessionEnd ends. Do not launch probe
   sessions or tabs to test; use the CLI against a temporary database and the real hook payload fixtures.

## Open decisions for that session

- Whether `sessions.db` is archived or deleted after import.
- Whether session deletion (today's `session_close.py` path) is still a requirement.
