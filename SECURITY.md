# Security and credentials

This starter is a single-operator, trusted-local application. The HTTP listener
binds to 127.0.0.1. Do not expose it through a public reverse proxy or share its
operator key as a substitute for individual user accounts.

Each user authenticates their own Codex CLI and GitHub CLI. The repository contains
no working credentials. Generated operator/signing keys and the database belong
in the private runtime directory outside this checkout. Never put credentials in
ticket text, screenshots, issue reports or Git commits. The local operator key is
shown only when the user explicitly asks to reveal it; it is not a GitHub/API key.

Coder and QA run with a named native workspace sandbox and restricted inherited
environment. GitHub publication is a separate host operation. Native execution
is not a VM or a security boundary against a hostile process running as the same
OS user. Review repositories, toolchains, dependencies and task scope before
resuming execution. A Ready engineering ticket can authorize a PR and, when its
delivery policy allows, a merge. Choose manual merge when human approval is needed.

Pause prevents new launches; Stop requests termination. An uncertain outcome
holds the repository lane until reconciliation proves what happened. Do not
remove locks, edit SQLite, or retry blindly to clear an uncertain operation.

## Reporting a vulnerability

Use the repository's private vulnerability reporting feature if it is enabled.
If it is unavailable, open a public issue containing only a request for a private
contact channel. Do not include exploit details, tokens, personal paths, database
files, provider output or private source code in a public issue.

## Before publication

Run `python3 scripts/check-public.py`. This is a guard against common accidental
leaks, not a guarantee: review staged files and their history as well. Never
publish a runtime directory or import an existing machine's Git history as a
starter template.
