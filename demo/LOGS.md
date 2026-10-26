# Agent activity log

The **Logs** button beside Builder workboard opens an activity dialog without
stopping running agents. It shows timestamped role handoffs, context preparation,
generation, browser-check start/results, retries and errors. The current operation
shows elapsed time and time since the last runtime event. Expand a completed
response to inspect its source patch or JSON; a response is not a test pass or an
accepted change. **Follow latest** can be disabled while reading older entries,
and **Copy log** exports plain text. Escape/Close returns to the builder without
stopping work.

Generation uses the pinned SDK’s streaming API. Activity reports elapsed time,
actual received message updates and time without new output. These updates are
not token counts or completion percentages, and hidden reasoning is never shown.
The first build uses shorter generation; repairs can use extended reasoning.
Before the first output, the log says it is waiting. See [README.md](./README.md)
for the measured performance comparison and fixed deadlines. Logs are
bounded to 200 entries (6,000 characters of detail each), held only in memory and
not written to IndexedDB on each update. Recorded mode labels historical events;
reload retains the existing task milestones but not detailed session telemetry.

Run the additional checks with `node --test tests/run-log.test.mjs` and
`python3 tests/run-log-browser.py`. The browser component test uses explicit
model fixtures, not an actual GPU inference run or full-app integration test.

When a run stops with an error, **View error log** appears beside the notice and
in the blocked task. It opens the current log with the latest output expanded.
After a reload, the same link restores bounded saved model outputs and the task
error; earlier transient runtime events are not recovered. Unsupported check
properties and operators identify the check, assertion number, and supported
values so the model can correct its plan. Native form controls support boolean
`disabled` checks, including browser-defined fieldset and optgroup inheritance.

Plans for visible or behavioral changes must include a requested-outcome check
that fails on the current app. A plan whose checks all already pass receives one
correction with bounded browser observations; it cannot proceed unchanged.
Explicit behavior-preserving refactors can use `verification:"preserve"`, shown
in the plan as preserving existing behavior; all baseline checks may pass.

The single Planner correction enables bounded reasoning (4,096 output tokens),
while its initial plan stays at 2,048 tokens without extended reasoning. The
correction receives the precise validation error and bounded browser facts, not
the rejected raw JSON. Raw completed responses stay available in the error log.
