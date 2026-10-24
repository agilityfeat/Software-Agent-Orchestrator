# Setup

The normal path uses local native Codex sessions for Main, Coder and QA. It needs
no Docker daemon, provider proxy, private image or companion repository.

## Accounts

Authenticate GitHub CLI using `gh auth login --hostname github.com`. The account
must have push access to a ticket's exact repository. Organizational policies and
branch protection may still require human approval. Never weaken those settings
merely to make automatic merge work.

Authenticate Codex CLI using `codex login`. The starter references the user's
own private Codex authentication file. The current starter expects ChatGPT
account-based CLI login; API-key-only authentication is not supported by the
bootstrap. It does not ship credentials or copy an
existing developer's account. A separate Codex home can be used:

```sh
CODEX_HOME="$HOME/.codex-orchestrator" codex login
./orchestrator init --codex-home "$HOME/.codex-orchestrator"
```

Protect credential directories and files from other local users. If setup reports
unsafe permissions, correct the exact named directory/file and rerun it. Do not
make your whole home directory readable or grant the agent unrestricted access.

## State and workspaces

The default runtime directory is outside this repository:

- macOS: `~/Library/Application Support/software-agent-orchestrator`
- Linux source path: `$XDG_DATA_HOME/software-agent-orchestrator`, falling back
  to `~/.local/share/software-agent-orchestrator`. Linux is not yet a validated
  supported execution platform.

State contains the database, keys, setup references and execution journals. It
is private application data, not an exportable project template. Do not commit
it or include it in a public issue.

`./orchestrator init --help` describes the explicit state, project, binary,
auth-home and read-root overrides. Use absolute paths. Repeating init for the same
configuration is safe; it does not silently rotate keys or overwrite a different
configuration. The `key` command requires an interactive terminal.

A new installation starts with an empty workspace catalog. The app verifies GitHub
identity before using a known local repository; otherwise it creates an isolated
clone under the configured project root. Coder works in a task workspace, and QA
uses a separate exact-commit checkout. Repositories are selected by ticket fields,
not by instructions embedded in arbitrary URLs or source text.

## Toolchains and permissions

Install a project's normal toolchain on the host before running its tasks.
Dependencies can be prepared by the coding agent within its sandbox. Native
agents do not inherit all of your shell configuration or account credentials.
If a legitimate compiler/SDK location is inaccessible, use the explicit read-root
setup option after reviewing the scope; do not approve the entire home directory.
Supplying `--read-root` replaces the default read-root list, so include every
intended toolchain root. Homebrew and Command Line Tools roots are the macOS
defaults when present.

Keep Codex at the documented compatible version. Executable contents are pinned:
a tool upgrade can require a deliberate setup refresh. A pin failure is a reason
to stop and review the installed tool, not to disable validation. See operations.

## Configuration validity

Generated execution setup is valid for one year and bounds each repository to
10,000 Runs. It does not silently renew itself or reset a spent budget. This
first release has no automatic update/renew command: preserve state and follow
a reviewed upgrade procedure before changing the pinned toolchain.

## First run

Run `doctor`, then `start`; reveal the local key using the explicit `key` command
in a second terminal. Open the loopback URL, review System and resume launches.
Use a small task with manual merge first. Neither installation nor `doctor`
creates a provider request, ticket or PR.
