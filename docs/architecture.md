# Architecture

The application is a Rust crate embedding its HTML, CSS and JavaScript. SQLite
stores tasks, revisions, command receipts, plans, agent settings, Runs and events.
The UI uses the authenticated HTTP API and live activity stream. There is no
separate JavaScript server and no external scheduler in the normal starter path.

## Agent roles

Main proposes a bounded list of tasks from the supplied goal and context. A human
edits/selects proposals before acceptance into Backlog. Main has no repository
source access during planning. It cannot secretly turn planning into execution.

Coder receives an immutable task snapshot and isolated repository workspace. The
host verifies actual completed check commands rather than trusting test claims.
Required independent QA receives the exact implementation commit in a separate
workspace and must leave source unchanged. Repair feedback is scoped to the task;
unrelated findings become Backlog bug tickets with criticality.

The host publisher owns GitHub writes and merge observation. A coding agent does
not receive the publisher's GitHub token. Manual merge remains a veto over an
automatic merge path. Skipping QA is explicit in the Run and delivery evidence;
it never means QA passed.

System settings define instructions, models and role defaults. Edits apply to
new admissions; existing Runs retain their frozen settings. Defining more agents
does not grant additional filesystem paths, executables or credentials.

## Queue and failure semantics

Each repository has one active lane. Different repositories can run concurrently.
Ready work is selected by priority, subject to completed prerequisites. A failure
pauses that repository; other repository queues can continue. Pause fences new
launches. Stop requests cleanup. Service restart never blindly replays uncertain
external actions; reconciliation must establish their outcome.

The application includes deadlines, durable receipts, finite repairs, private
local browser sessions and offline snapshots. These are execution mechanics, not
a claim that all arbitrary repositories or SDKs have been validated.

## What this starter does not include

- Multi-user accounts, roles, shared tenancy or public hosting.
- Slack agents, CRM/email integrations or business-event intake.
- Persistent conversational memory or company-specific specialist context.
- Recurring business schedules, deployment automation or host maintenance.
- Arbitrary provider support. The native adapter targets the documented Codex
  CLI version and its execution-event/permission contract.
- Full provider transcripts or raw command logs in the UI. Evidence intentionally
  retains finite summaries and verification facts instead of private output.

Some low-level compatibility modules and optional container profiles remain in
source. They are not required by the native starter path and do not install or
activate any external runtime. Container assets use their own explicit build
context; do not add credential directories to it.

## Repository-free research

A Research ticket with an empty repository freezes `kind=research` into its Run.
Repository-backed Research keeps the coding workflow. There is one dedicated
research lane alongside repository queues: uncertain research holds that lane
without blocking unrelated repository work. Priority/oldest Ready/dependencies,
immutable task/agent settings, one admission per revision, pause and deadlines
remain enforced. Stop acknowledged before report persistence wins; restart
requires cleanup/reconciliation and never automatically reruns the search.

The signed native-only research policy bypasses repository discovery, Git,
validation profiles and publisher. It uses an empty read-only scratch directory,
no known-workspace/toolchain read roots and disabled local tool networking. The
pinned CLI receives `--search` before `exec`, enabling provider-side built-in web
search. Actual `item.completed` / `web_search` events and a completed turn are
required; shell/file/MCP tools are rejected, including started forbidden items.
The event contract is documented in the pinned
[Codex0.153.4 source](https://github.com/openai/codex/blob/rust-v0.153.4/codex-rs/exec/src/exec_events.rs).
A completed search proves invocation/results returned, not that every source was
read or every claim is true. Reports must state evidence and limitations.

Reports are strict structured Markdown (8000bytes) and up to8 unique HTTP(S)
source URLs/titles, at most12000serialized bytes. No model-selected filenames or
arbitrary filesystem uploads are accepted. The authenticated UI displays text,
and successful Runs expose generated attachment downloads at
`GET /api/workboard/v1/runs/<id>/report.md` and `.../sources.json` with fixed
content types and `nosniff`. Private report data persists with the Run/receipt.

Research defaults to Skip QA, truthfully displayed as skipped. Required QA uses
a distinct session to review the exact SHA256 of the structured report. Concrete
bounded findings can return the same Run to research for at most2 repairs under
the original deadline. Unknown/transport/auth/malformed results never authorize
repair or Done. No Git SHA, test pass or PR is fabricated. Original approvals
remain the cleanup authority even after host settings change or expire.

Main may plan a goal with no repository; only Research proposals can be accepted
from that plan. Acceptance still creates manual Backlog tickets and never starts
implementation; Ready explicitly enrolls them. Coding proposals require a repo.
