# Workflow exceptions

Each zizmor exception names one exact rule, workflow, line and column.
Job naming exceptions in `workflow-baseline.json` preserve published check contexts.

| Rule | Location | Reason |
| --- | --- | --- |
| `self-repository` | `ci.yml:35:11` | The local reusable workflow syntax is supported by actionlint and existing GitHub runners; keep the existing invocation contract. |
| `dangerous-triggers` | `release.yml:4:3` | Release qualification requires a successful own-repository main push; only that qualified commit is built, and no pull-request code or downloaded scripts run with release permissions. |
| `undocumented-permissions` | `release.yml:62:7` | Tommy forbids explanatory code comments. The exact permission grants the named qualification, status-reading or release-publication job its required API access. |
| `self-repository` | `release.yml:52:11` | The local reusable workflow syntax is supported by actionlint and existing GitHub runners; keep the existing invocation contract. |
