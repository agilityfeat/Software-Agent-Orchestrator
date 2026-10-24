# Local operation

Run `./orchestrator doctor` before starting or after upgrading a prerequisite.
Run `./orchestrator start` in a terminal; Ctrl+C requests graceful shutdown.
The starter installs no background service. You control when it is running.

Use the same `--state-dir` on every command if you chose a non-default instance.
A remembered browser session does not mean queue execution is resumed: check the
launch state in the UI.

## Pause, stop and reconcile

Pause blocks new stage launches. It does not claim an existing process stopped.
Use a task's Stop control when termination is needed, then observe cleanup.
If an operation is uncertain, use Reconcile to inspect its recorded outcome.
Do not delete a lane, edit SQLite or create a duplicate ticket to force a retry.

A failed task pauses its repository. Fix the stated cause, inspect the task and
resume the repository deliberately. Other repositories can continue independently.
Changing an agent's instructions or model applies to future admissions; it does
not rewrite an already admitted Run.

## Backup and restore

Pause launches, settle every active/uncertain Run and pending Main generation,
then stop the process. Snapshot and restore deliberately refuse unresolved work.
The commands below use placeholders: substitute canonical absolute paths.

```sh
target/release/workboard-rs snapshot /absolute/private/current.sqlite /absolute/private/snapshot.sqlite
target/release/workboard-rs restore /absolute/private/snapshot.sqlite /absolute/private/restored.sqlite
```

Build the offline utility with `cargo build --locked --release` if you used
`init`'s private build directory and do not have `target/release` in this checkout.

Both source and destination must be in private directories. The destination must
be new. SQLite's backup API includes committed WAL data. Do not copy just a live
`.sqlite` file or replace the database beneath a running process.

Snapshots preserve board/history state; they do not undo GitHub merges or replace
provider credentials. Keep private setup references, keys and action journals
securely with your recovery plan. Never publish a backup. Never enable both the
original and restored instance against the same work concurrently.

## Updates

Pause, settle work, stop and snapshot first. Keep the working executable and its
private configuration until the update is verified. Pull a reviewed release and
build with `cargo build --locked --release`; executable pins intentionally detect
changed binaries. Do not manually edit pin hashes or private SQLite state to
silence a mismatch. The starter currently refuses silent setup replacement;
configuration/tool upgrades need an explicit reviewed refresh. There is no
background updater or automatic cross-schema downgrade.

This is an early local starter, not a zero-maintenance production service. Report
setup errors using finite error messages and tool versions, without attaching
authentication files, databases, private repository contents or raw provider logs.
