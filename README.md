# Software Agent Orchestrator

**Run an AI development team locally, or try the workflow in your browser.**

This repository has two parts: a **local development application for real projects**
and a **browser demo that showcases the idea with a small Google Gemma 4 model**.
They demonstrate the same idea: **plan → code → check → review**.

[Try the browser demo](https://agilityfeat.github.io/Software-Agent-Orchestrator/) ·
[Set up the local application](#local-ai-development) ·
[Documentation](#documentation)

| | Local AI development | Browser demo |
|---|---|---|
| **Purpose** | Work on your own software repositories | Explore agent orchestration on a small sample app |
| **Runs where?** | On your computer, with a local web interface | In your browser, served through GitHub Pages |
| **AI** | Codex CLI using your account | Google Gemma 4 E4B downloaded to your device |
| **Output** | Tested code, a GitHub PR and optional merge | An edited HTML/CSS/JavaScript app saved in your browser |
| **To start** | Install the tools and sign in to your accounts | Open the demo; no account or API key required |

**The demo is a showcase, not the full development application running in a browser.**
“Local” describes where the development tool runs, not offline Codex inference.
The demo's Gemma model does run on your device.

## Local AI development

**Main** proposes tasks from your goal and context. **Coder** inspects the
repository, implements a task and runs checks. A separate **QA** session reviews
the same code version and can request a limited number of repairs. The result is a
GitHub PR, with manual or automatic merge according to the task's policy.

The workboard tracks dependencies, priorities, repository queues and results,
with pause/stop controls. **Research** tasks without a repository can instead
deliver a report with sources.

### Install and start

The local app is an early, **single-user tool**, initially validated on macOS. Windows uses
[WSL2 Ubuntu](docs/windows.md), not native Windows; physical Windows validation
is still pending.

You need Git and platform build tools, **Rust/Cargo 1.85+**, **Python 3.11+**,
**GitHub CLI (`gh`)**, and **Codex CLI 0.153.4**. Keep the compatible Codex version
and install your project's toolchains. No Docker, Node server or frontend build
is required for the normal setup.

```sh
git clone https://github.com/agonza1/Software-Agent-Orchestrator.git
cd Software-Agent-Orchestrator

gh auth login --hostname github.com
codex login

./orchestrator init
./orchestrator doctor
./orchestrator start
```

Use your own accounts; setup expects account-based Codex login, not API-key-only
authentication. Advanced configuration is in the [setup guide](docs/setup.md).

Open **<http://127.0.0.1:4311/#work>**. In a second terminal, from the same checkout:

```sh
./orchestrator key
```

Paste the generated operator key into the **local app only**. Never share it in a
ticket, chat, screenshot or commit. Setup stores private state outside the
repository and starts with execution **paused**. Stop the server with Ctrl+C.

### Run your first task

1. Review agents in **System**, leave model fields blank for the CLI default,
   then resume launches.
2. Create a small task with an `owner/repository` you can push to, an objective
   and acceptance criteria. Start with **Required QA** and **Wait for manual merge**.
3. Move **Backlog → Ready**, follow the work and checks, then review the PR.

Or ask **Main** to propose tasks, accept them into Backlog, then move them to
Ready. Manual workflow tasks do not start agents.

**Ready authorizes execution.** Automatic merge can change your default branch;
merging does not authorize deployment. Keep the app local; do not expose it to
the public internet. Read the [security boundaries](SECURITY.md) before enabling automation.

## Browser demo

[**Open the demo**](https://agilityfeat.github.io/Software-Agent-Orchestrator/)

A familiar expense app sits beside a builder workboard. In **Live** mode,
Planner, Coder and Tester share one small **Google Gemma 4 E4B** model through
separate conversations. They really edit HTML/CSS/JavaScript and run browser
checks; you approve the result.

1. Open the page and click **Load Gemma**. Use a WebGPU-capable desktop browser;
   the first download is approximately **3 GB**, then cached when storage allows.
2. Request a small change, such as **“Add a date column to the table.”** Review the
   plan and choose **Approve plan & run**. Open **Logs** to follow activity.
3. Compare **Candidate** with **Accepted**, review the checks, then **Accept change**.

No installation, backend, account or API key is needed. It does **not** access
your repositories, create GitHub PRs or deploy changes. Results stay in that
browser; **Export workspace** saves a copy.

**Guided** needs no model download: scripted agents, real browser checks.
**Recorded** shows a successful Live run saved in that browser. The separate
**Developer workboard** link is a read-only preview of the local tool's UI.

The demo is not a benchmark of the full tool's capabilities or speed. See the
[demo guide](demo/README.md) for local hosting, requirements and limitations.

## Documentation

| Need help with… | Guide |
|---|---|
| Accounts, toolchains and private state | [Local setup](docs/setup.md) |
| Windows installation | [WSL2 setup](docs/windows.md) |
| Updates, pause/stop and recovery | [Operations](docs/operations.md) |
| How the local application works | [Architecture](docs/architecture.md) |
| Running or extending the browser demo | [Demo documentation](demo/README.md) |
| Contributing, tests and publishing safely | [Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) |

## License

[MIT](LICENSE): personal and commercial use, modification and redistribution are
allowed with the required notices. Provided without warranty. Model, dependency
and account-provider terms still apply.
