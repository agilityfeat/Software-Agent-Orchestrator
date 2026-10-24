# Local operation

Run `./orchestrator doctor` before starting or after changing a prerequisite,
then `./orchestrator start`. Ctrl+C requests graceful shutdown. No background
service is installed. Use the same `--state-dir` on every command for a custom
instance, and check the UI launch state: remembered sign-in does not resume work.

## Pause, stop and reconcile

- **Pause** blocks new stages; it does not terminate an active process.
- **Stop** requests termination. Check that cleanup completes.
- **Reconcile** checks an uncertain outcome before any retry.

Never edit SQLite, delete a lane or duplicate a ticket to bypass recovery.
A failed coding task pauses its repository; fix the cause and resume deliberately.
Other repository queues can continue. Uncertain research holds its own lane.
Agent instruction/model edits apply to new Runs, not an already admitted Run.

## Backup and restore

Pause launches, settle active/uncertain Runs and pending Main generations, then
stop the process. Snapshot and restore refuse unresolved work. Substitute
canonical absolute paths in these commands:

```sh
target/release/workboard-rs snapshot /absolute/private/current.sqlite /absolute/private/snapshot.sqlite
target/release/workboard-rs restore /absolute/private/snapshot.sqlite /absolute/private/restored.sqlite
```

If `init` built into its private directory, build the offline utility here with
`cargo build --locked --release` first. Both paths need private parent directories,
and the destination must be new. SQLite's backup API includes committed WAL data;
do not copy a live `.sqlite` file alone or replace one beneath a running process.

Snapshots preserve board/history state, not provider credentials or external
GitHub effects. Keep setup references, keys and action journals in your private
recovery plan; never publish backups. Never run original and restored instances
against the same work concurrently.

## Updates

Pause, settle, stop and snapshot first. Retain the working executable and private
configuration until the update is verified. Build a reviewed release with
`cargo build --locked --release`; changed executable pins require an explicit
reviewed configuration refresh. Do not edit hashes or SQLite to silence errors.
The starter has no silent setup replacement, background updater or automatic
cross-schema downgrade.

Report errors with finite messages and tool versions. Do not attach authentication
files, databases, private repository contents or raw provider logs.
