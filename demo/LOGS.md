# Agent activity log

The **Logs** button beside Builder workboard opens an activity dialog without
stopping running agents. It shows timestamped role handoffs, context preparation,
generation, browser-check start/results, retries and errors. The current operation
shows elapsed time and time since the last runtime event. Expand a completed
response to inspect its source patch or JSON; a response is not a test pass or an
accepted change. **Follow latest** can be disabled while reading older entries,
and **Copy log** exports plain text. Escape/Close returns to the builder without
stopping work.

Generation still uses the existing non-streaming API and model settings. During
that call the log says it is waiting; it does not fabricate token counts, percentage
complete, or reasoning. This improves visibility, not inference speed. Logs are
bounded to 200 entries (6,000 characters of detail each), held only in memory and
not written to IndexedDB on each update. Recorded mode labels historical events;
reload retains the existing task milestones but not detailed session telemetry.

Run the additional checks with `node --test tests/run-log.test.mjs` and
`python3 tests/run-log-browser.py`. The browser component test uses explicit
model fixtures, not an actual GPU inference run or full-app integration test.
