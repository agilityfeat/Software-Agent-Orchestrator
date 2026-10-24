# Software Agent Orchestrator

**Your AI team, from goal to delivery.**

A lightweight, local-first AI agent workspace. Define a goal, review Main's proposed
tickets, and let Coder and independent QA take repository work through checks,
repairs, a pull request and optional merge. The Kanban board shows what is queued,
active, blocked and delivered.

One Rust application, one SQLite database, an embedded browser UI and your own
Codex and GitHub accounts. No n8n, Node server, frontend build, private service,
or Docker image is required for the normal starter path.

This is an early **single-user local starter**. macOS is the initial validation
platform. The source uses Unix APIs; Linux execution/sandbox compatibility needs
separate validation. Windows is not supported. This is not a hosted multi-user
service. Setup is pinned for one year and refuses silent tool/configuration
replacement; unattended upgrades and renewal are not included in this first release.

## What is included

- **Main:** turn a goal into editable proposals, with acceptance criteria,
  priorities, labels and dependencies. Review and accept proposals into Backlog.
- **Workboard:** compact filters, editable tickets, inline priority and drag
  Backlog ↔ Ready. Ready engineering tickets enter their repository's queue.
- **Continuous processing:** one active task per repository, with different
  repositories processed in parallel. A failed task pauses its own repository.
- **Coder → QA:** independent review of the exact implementation commit, bounded
  repair attempts, and separate Backlog bug tickets for unrelated findings.
- **GitHub delivery:** create a PR and either merge after successful checks or
  wait for a human merge, according to the ticket's delivery policy.
- **System:** editable Main, Coder and QA agents, additional role-based agents,
  instructions, model choices and default assignments.
- **Operator controls:** pause, stop, reconcile uncertain outcomes, retained
  activity and remembered local sign-in.

Main currently plans from the goal and context you provide; it does not inspect
repository source. Coder discovers the repository and performs implementation.
A Research ticket defaults to skipping independent QA, but Coder's relevant
checks and delivery evidence remain required. Research still uses the repository
workflow; this starter does not publish reports to external document systems.

## Quick start

Prerequisites:

- macOS with Command Line Tools and Git.
- Rust/Cargo **1.85 or newer**, Python **3.11 or newer**, GitHub CLI (`gh`).
- Codex CLI **0.153.4**, authenticated with your own account. The execution-event
  and permission-policy contract is version-specific; newer versions need testing.
- Any language toolchains required by your actual repositories, such as Node,
  Python or Rust. Node **20+** is needed only for the UI contributor tests.

For CLI installation guidance, see [Codex CLI](https://github.com/openai/codex),
[GitHub CLI](https://cli.github.com/) and [Rust](https://www.rust-lang.org/tools/install).
If you use npm to install Codex, select this starter's compatible version with
`npm install -g @openai/codex@0.153.4`. Setup resolves the package's native binary
without running its JavaScript launcher.

The agent's model must be available to your own Codex account/CLI. Leave the model
blank in System to use the CLI default.

```sh
git clone https://github.com/agonza1/Software-Agent-Orchestrator.git
cd Software-Agent-Orchestrator

# Authenticate your own accounts. Never paste credentials into this repository.
gh auth login --hostname github.com
codex login

./orchestrator init
./orchestrator doctor
./orchestrator start
```

Open **<http://127.0.0.1:4311/#work>**. In another terminal, inside this checkout:

```sh
./orchestrator key
```

This explicit command reveals the generated local operator key in your terminal.
Paste it into the app once; browser sign-in is remembered. Do not paste that key
into a ticket, issue, chat, screenshot or Git commit. It is distinct from your
GitHub and Codex credentials. Stop the foreground server with Ctrl+C.

`init` builds the Rust binaries, creates private state outside the checkout, generates new local keys and
records the installed executable pins. It does not import this author's database,
agent settings, workspace catalog, credentials or Git history. The first launch
is **paused**: configure System, then explicitly resume launches in the app.

Choose a different port or dedicated account directory on your **first** init
when needed (existing settings are deliberately not overwritten):

```sh
./orchestrator init --port 4312 --codex-home "$HOME/.codex"
./orchestrator init --help
```

For a separate instance, supply the same absolute `--state-dir` to **every**
command (`init`, `doctor`, `start`, `key`). See [setup details](docs/setup.md).

## Complete a first task

1. Open **System** and review Main, Coder and QA instructions. Keep model fields
   empty initially unless you know your account supports the chosen model.
2. Resume launches. Create a small ticket with an exact `owner/repository`, a
   concrete objective and acceptance criteria. Use a repository your own GitHub
   account can push to.
3. Choose **Wait for manual merge** for your first ticket. Required QA is the
   default for engineering work.
4. Move the ticket to **Ready**, or drag its card there from Backlog. The app
   verifies repository access, prepares an isolated workspace and queues the work.
5. Follow activity and the PR link. Coder runs relevant checks; a separate QA
   session reviews the exact commit. In-scope failures can return to Coder within
   the repair limit. A human-merge ticket completes after its merge is observed.

Alternatively, enter a goal under **Main**, review the proposals and accept the
ones you want. Accepted proposals start in Backlog; move them to Ready when you
want execution. A Manual workflow ticket tracks work without starting an agent.

**Ready is an execution decision.** Automatic-merge tickets may change the
repository's default branch after the required checks pass. The app never grants
deployment permission merely because a ticket merges.

## Credentials and privacy

**No usable credentials are included.** Each installation uses its owner's own
accounts and newly generated operator/signing keys. Runtime state stays outside
Git. GitHub credentials go to repository-access/publication operations, not to the
coding model's inherited environment. Codex authentication is handled through the
private local auth reference and isolated per-action homes.

The app listens on loopback. Keep it local; shared accounts, public hosting and
internet exposure need a different access model. Native agents use an OS sandbox,
not a VM. Read [security boundaries](SECURITY.md) before enabling automation.

Before publishing changes:

```sh
python3 scripts/check-public.py --history
```

The scanner checks candidate files and all local Git refs for common credential
patterns and private runtime artifacts. It never prints matched secret values.
A clean scan is supporting evidence; it does not replace whole-diff review.

## Architecture and operation

```mermaid
flowchart LR
    Goal[Goal] --> Main[Main: propose tasks]
    Main --> Review[Human review]
    Review --> Board[Backlog / Ready]
    Board --> Queue[Per-repository queue]
    Queue --> Coder[Coder + checks]
    Coder --> QA[Independent QA]
    QA -->|In-scope repair| Coder
    QA --> Delivery[PR / optional merge]
    Delivery --> Done[Done]
```

Research or explicitly skipped-QA tickets use Coder checks followed by delivery.
The Rust coordinator owns revisions, stage transitions and publication evidence;
a model's claim alone cannot mark a task delivered.

- [Setup and account configuration](docs/setup.md)
- [Start, update, pause and recover](docs/operations.md)
- [Architecture and limitations](docs/architecture.md)
- [Contributing and local tests](CONTRIBUTING.md)
- [Security](SECURITY.md)

MIT licensed. This standalone starter includes generic application source only;
it does not include a personal operating environment or the broader business
integration system from which it was extracted.
