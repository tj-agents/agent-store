# agent-store

Open-source Rust library and JSON CLI, edition 2024, toolchain `1.97.1`. The `rust:*` and `seaorm:*`
standards apply. `README.md` is written for outside users: keep tj-agents, machine and employer specifics
out of the crate, its defaults and the README.

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

<!-- BEGIN personal:rules sha256:c51fe768beb91b7db22fa02bf4bcffd659f1ec23eee5cf37f2bd5eedfe59aba3 -->
# Personal rules

These rules govern every project that explicitly adopts `personal@tj-agents`. Adoption records the
project's personal scope; a home directory, author name or employer association never establishes it, and
employer projects stay outside this profile unless the user explicitly extends that scope. Before planning
or starting work, confirm the task is clear against each rule below.

## 1. Clean cut

Keep one current supported solution. When replacing behavior, remove the superseded names, aliases,
fallback implementations, adapters, configuration formats, compatibility branches and forwarding or
migration docs. Migrate all known callers, project declarations, installed registrations, generated
artifacts and live runtime entry points in the same cutover, and coordinate upgrades with the actual
collaborating developers. Run each migration once during the cutover and delete it; never ship migration
or cleanup code that runs again later. CI fails a commit that removes or renames a skill while a tracked source
still names its old `<package>:<name>`.

Verify the entry points people actually use on every supported platform, including Windows and Linux
launch paths where supported. Source edits, successful generation, passing unit tests and version commands
alone leave a migration unproven; check installed bindings and fresh runtime behavior. Record any
unverified entry point as incomplete, with its resolver and observable acceptance condition.

```text
Replace old-launch with launch:
1. Migrate the registered shell command, configuration and actual callers.
2. Delete the old entry and regenerate installed settings.
3. Invoke launch from a fresh supported Windows/Linux session and check its actual behavior.
Result: launch --version alone leaves this migration incomplete.
```

A backwards-compatibility exception exists only as an explicit user requirement. Surface it as a
deliberate policy decision, identify the affected supported behavior and reconcile this profile's scope
before implementing it, keeping the exception visible rather than inside a fallback or adapter.

## 2. Proportionate implementation

Solve the demonstrated problem with the smallest suitable implementation. Add abstractions, dependencies,
configuration switches, duplicate paths or process machinery only for a concrete current need. Prefer the
capabilities the project already selects. Verify the behavior and meaningful failure paths in proportion
to the change, with tests that prove behavior rather than restate the implementation.

## 3. Established problem records

Owns automatic recording of established personal bugs, agent or process mistakes, and deliberate debt.
Within authorized work in an explicitly adopted personal project, record them through Personal's selected
tools without waiting to be asked. Honor an explicit no-recording limit, and never record employer or
non-personal evidence through Personal.

Reconcile the canonical existing identity before recording. For a repeated observation, append material
new recurrence evidence, the cause when known, and the current outcome or repair state; do not create a
duplicate item. Verify remote persistence before claiming the problem is documented. The existing Personal
tools own their commands and procedures.

Recording does not authorize, prioritize or dispatch implementation. Apply
[Workflow's incidental-problems decision](https://github.com/tj-agents/core/blob/main/.agents/workflow/resources/problem-records.md#incidental-problems)
to those decisions and preserve the active task and repair owner.

```text
An acknowledged repeated native close prepare failure occurs during item A:
1. Update its existing close issue with new recurrence evidence, the known cause and current repair state.
2. Verify the update persists remotely before saying it is documented.
3. Continue item A; do not create a duplicate item or take over the repair.
```

## 4. Plans

Track work as a GitHub issue or project item. A plan committed under `plans/` is only the detailed
implementation plan behind one such item and links to it; working plans stay machine-local.

This policy is authored once here. A selecting project receives it at SessionStart, and adoption generates
its marked native instruction block from this body and checks it for drift. A selected plugin or an
`always` tier alone is not proof of loading.
<!-- END personal:rules -->
