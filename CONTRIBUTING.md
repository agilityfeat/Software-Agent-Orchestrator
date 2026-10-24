# Contributing

Keep the default path small: one local Rust process, SQLite, an embedded UI and
user-owned agent credentials. Favor complete useful behavior over new services.

Use Rust 1.85 or newer, Python 3.11 or newer, and Node 20 or newer for UI tests.
No Node server or frontend bundler is required to run the app.

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
node --test tests/ui-model.test.mjs
python3 -m unittest discover -s tests -p 'test_*.py'
python3 scripts/check-public.py
```

The integration fixtures simulate Codex, GitHub and command execution. Running
tests must not require real credentials or create live PRs. State limitations in
PRs; synthetic adapter coverage is not a claim of provider compatibility on every
OS or toolchain. Do not weaken the execution sandbox or independent QA to make a
test pass. No CI service or external review bot is required for contributions.

To include the real local-server onboarding test (still using synthetic accounts,
with no model or GitHub calls), build the host binaries and supply their directory:

```sh
cargo build --locked --bins
ORCHESTRATOR_TEST_BIN_DIR="$PWD/target/debug" python3 -m unittest discover -s tests -p 'test_*.py'
```

Without that environment variable, the optional server test explicitly skips.
The remaining bootstrap tests use compiled synthetic command fixtures.
