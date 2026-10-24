# Software Agent Orchestrator

This repository is a standalone local AI agent workspace. Keep changes generic:
never add personal credentials, runtime databases, logs, account identifiers,
private configuration, company context or machine-specific paths. Runtime state
belongs outside the checkout. Tests use synthetic accounts and repositories.

The Rust crate embeds the UI. Keep Main, Coder and independent QA defaults,
revision checks, per-repository queues, bounded repairs, exact-commit verification,
manual merge vetoes and private local sessions intact. The normal starter must
not depend on another repository, a private image, n8n, or a preconfigured host.

Delegate bounded implementation to Coder and final independent review to QA.
Review the whole diff, run appropriate local tests, and run the public-export
scanner before pushing. Do not add or depend on CI or external Codex Review.
Never run paid provider calls, create real project tasks or merge test PRs merely
to validate setup. Publication does not authorize deploying this app or changing
another project's runtime.
