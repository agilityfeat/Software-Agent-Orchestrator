# Architecture

One Rust application embeds the UI and serves its authenticated HTTP API and
activity stream. SQLite stores tasks, revisions, plans, agent settings, Runs and
receipts. The normal starter needs no separate JavaScript server or scheduler.

## Planning and execution

Main proposes tasks from the supplied goal and context; it cannot inspect repo
source while planning or start execution by accepting a plan. Keep goals split
into small, independently verifiable tickets with clear acceptance criteria. Use
dependencies only for real prerequisites and a shared goal label instead of an
epic. Review and edit proposals into Backlog, then move chosen tickets to Ready.

Coder receives a frozen task and agent configuration in an isolated workspace.
The host verifies completed check commands rather than accepting test claims.
Required QA uses a separate session and reviews the exact implementation commit
without changing its source. In-scope findings can request bounded repairs;
unrelated bugs become manual Backlog tickets with criticality.

The publisher alone receives GitHub write credentials. It verifies the exact
reviewed commit and delivery policy; manual merge remains a veto. Skipped QA is
recorded as skipped, never passed. System edits affect new admissions, while
active Runs retain their settings. Agent configuration cannot grant more paths,
executables or credentials.

## Queues and recovery

Each repository has one active lane; different repositories can run in parallel.
Ready tasks run by priority and oldest Ready time, after their prerequisites are
Done. Failures pause the affected repository. Research has a separate single lane,
so uncertain research does not block repository queues.

Pause fences new stages; Stop requests cleanup. Restart never replays uncertain
work automatically. Reconciliation must establish the outcome before retrying.
Deadlines, immutable snapshots, durable receipts and finite repair budgets remain
in force throughout a Run. See [operations](operations.md) for recovery and backups.

## Repository-free research

Research with a blank repository produces a report, not a checkout, PR or merge.
Research with a repository retains the coding workflow. Main can also propose
Research-only tickets when its goal has no repository.

The native agent uses an empty read-only workspace, no project/toolchain read
roots and no GitHub credentials or publisher. Local-tool networking is disabled;
built-in web search is available. Completion requires a completed search and
turn, valid report data and confirmed process/auth cleanup. Shell, file and MCP
actions are rejected. A completed search does not prove every source was read or
every claim is true; reports must distinguish evidence from uncertainty.

Reports contain Markdown up to 8000 bytes and at most eight unique HTTP(S) sources,
within a 12000-byte structured result. The authenticated UI renders report text
and serves generated `report.md` and `sources.json` downloads bound to its digest.
There are no arbitrary agent-selected file uploads. Research defaults to Skip QA;
Required QA uses a distinct session to review the exact report digest and may
request up to two repairs under the original deadline. Invalid or uncertain
results never authorize repairs or Done, and an acknowledged Stop wins before
report persistence.

## Scope and security

This is a local single-user starter, not a shared or publicly hosted service.
Private sessions, scoped credentials and the pinned Codex sandbox/event contract
are required; passing checks does not imply every repository or SDK is supported.
Evidence retains bounded summaries rather than raw provider or command logs.
Offline snapshots preserve state but cannot undo external effects.

Slack/CRM intake, recurring schedules, deployment automation, persistent company
memory and arbitrary provider support are outside this starter. Optional legacy
container profiles remain in source but are not needed by the native path and
never authorize activation. Keep credentials out of their build contexts.
