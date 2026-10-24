# Software Agent Orchestrator

Keep this standalone app generic. Never commit credentials, runtime databases,
logs, account identifiers, private configuration, company context or
machine-specific paths. Runtime state belongs outside the checkout; tests use
synthetic accounts and repositories.

The Rust crate embeds the UI. Preserve Main, Coder and independent QA roles,
revision checks, per-repository queues, bounded repairs, exact-result verification,
manual merge vetoes and private sessions. The normal starter must not require
another repository, a private image or a preconfigured host.

For Main planning, favor small, independently verifiable tickets with clear
acceptance criteria. Use dependencies only for real prerequisites and a shared
goal label rather than a dedicated epic. Review proposals into Backlog, then
move selected tickets to Ready. Main plans from supplied context, not repo source.

Delegate bounded implementation to Coder and final independent review to QA.
Review the whole diff, run appropriate local tests and the public-export scanner
before pushing. Do not add CI or require external Codex Review. Never run paid
provider calls, create real project tasks or merge test PRs merely to validate
setup. Publication does not authorize deployment or another project's runtime
changes.
