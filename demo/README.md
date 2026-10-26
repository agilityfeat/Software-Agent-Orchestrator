# Agents at Work: full app workspace

The browser demo is a small, editable single-page app beside an agent workboard.
Live Gemma reads and edits the complete HTML, CSS and JavaScript. Requests are not
routed into appearance or expense-policy presets. For example, ask **Let's add a
date in table.** or add a button with new behavior. The initial expense app is an
editable starting point, not an immutable business-rule oracle.

The footer opens a [short guide](./how-it-works/). This document describes the
implementation and verification. [LEGACY.md](./LEGACY.md) retains the previous
preset-based implementation notes.

## Run and use

Serve the repository with any static HTTP server and open `demo/` in a compatible
desktop browser. Python is only a static file server:

```sh
python3 -m http.server 8080 --bind 127.0.0.1
# Open http://localhost:8080/demo/
```

Guided is an explicitly scripted date-column example with actual browser checks.
Live loads Gemma 4 E4B on WebGPU: approximately 3 GB on first download, then the
browser's origin-specific model cache. It requires no Rust backend, Ollama, API
key or paid provider call. Recorded reviews a successful Live full-app run saved
in this browser; it does not perform new inference. Legacy recordings are never
relabeled as full-app evidence.

Describe a change, choose **Create plan**, review the proposed browser checks,
then **Approve plan & run**. Compare **Candidate** and **Accepted** and inspect
all three files in **Code**. **Accept change** reruns the reviewed checks before
promoting the exact bundle. It saves only in this browser. Export workspace
retains the accepted files, model responses, checks and run history, plus bounded
local timing and activity counters.

**Logs** opens the detailed live activity view without stopping agents. It shows
role handoffs, real model activity, completed responses and browser checks. See
[LOGS.md](./LOGS.md) for its bounded, in-memory diagnostic behavior.

**Start over** cancels the current plan/build/checks and lets you enter a new
request, preserving the accepted app, successful runs and downloaded model. An
idle loaded model stays ready; stopping active inference releases its worker, so
click **Load Gemma** again to initialize from the browser cache. **Reset demo**
also works during a run and asks before restoring the sample app and clearing
saved runs. Interrupted or blocked runs show their actual failure and recovery
action. Late results cannot revive a cleared task.

## Full-page source and checks

`page-baseline.mjs` contains the complete initial expense app and the honest Guided
example. `page-contract.mjs` validates and canonicalizes `{html,css,js}`. Coder
returns atomic exact-search edits, for example:

```json
{"edits":[{"file":"html","find":"<th>Amount</th>","replace":"<th>Date</th><th>Amount</th>"}]}
```

Each find must match exactly once in the current file; failed edits do not partly
change the workspace. Complete replacement files are also supported through
`{"files":{"html":"complete HTML"}}`. There is no selector, appearance-token or
business-function whitelist for app source. Workspace text is bounded (32,000
characters per file and 64,000 total); a growing app must still fit the local
model's context. Full source is supplied without silently omitting files.

Planner freezes concrete declarative browser checks for review. Coder sees the
current files and actual failures; Tester independently reviews the actual DOM
results and may propose extra data-only checks. The host executes them in real
browser DOMs. It never treats a model's pass flag as evidence. There are at most
two Coder repair cycles, with raw role responses retained for diagnosis.
The first build uses a short response without extended reasoning; repairs enable
reasoning again. Streaming activity reports elapsed seconds, actual received
updates and time without new output. It never shows invented completion
percentages or thought text. Each model step still has a fixed three-minute
deadline; progress does not extend it.

The check DSL supports arbitrary CSS selectors, click/input/select/key/wait
steps, and assertions on text, count, value, checked, attribute, visibility and
computed style. Node assertions pass when any matched node satisfies them;
`all:true` requires every match, and `first:true` inspects only the first. Count
checks count all matches and actions target the first. To count cells in one
row, use `tbody tr:first-child td`; `tbody tr td` counts the entire table. No matching node fails a
node assertion. Up to two checks run concurrently, each in its own new opaque
frame. Every top-level check starts in a fresh page with empty app
storage; its setup actions do not carry into another check. State-change checks
must declare their own actions, and contradictory single-observation outcomes
with identical actions are rejected before approval. Plans bind the check-semantics version; older evidence must be
replanned instead of being reinterpreted. Assertions are data, not executable test code. Regular expressions
use a bounded simple subset. Baseline results are retained for comparison; a
behavior-preserving refactor is allowed. Exact source no-ops, missing check
coverage, errors and failing assertions block acceptance. Source, plan and checks
are bound to SHA-256 evidence; acceptance rechecks the same bundle and rejects
stale or cancelled work. Acceptance reruns fresh candidate checks; it does not
repeat an unused baseline run.

## Isolated preview and persistence

The parent never evaluates generated app code in its own document. It uses the
same `page-runtime.mjs` and `preview/` runtime for visible previews and fresh
hidden test frames. Opaque sandboxed frames cannot read the workboard's storage,
credentials or DOM. A private MessageChannel is connected before app code starts;
trusted browser primitives are captured before the app can modify prototypes.
JavaScript module completion, errors and asynchronous timeouts are observed.

The preview supports browser HTML/CSS/JavaScript and public HTTPS scripts, modules
and CORS APIs under its content security policy. Use absolute/CDN module URLs;
relative imports inside inline module code resolve from its blob URL, and bare
npm package names need browser import maps. Server-only APIs, parent credentials,
parent navigation and external form submission are unavailable. Network requests
still follow browser CORS, device permissions and connectivity. A JavaScript
infinite loop can hang a shared browser renderer; timeout messages are diagnostic,
not a hard process-kill guarantee. Stop/reset or a browser reload may be needed.

Accepted and Candidate previews have separate identities and app-local storage.
Candidate starts with an isolated copy; acceptance preserves app data and remounts
the accepted role. Fresh checks cannot alter either visible session. Delayed
errors from the current candidate block acceptance; stale-frame callbacks are
ignored. Schema 3 stores full app bundles and full-page run history. Existing
schema 1/2 workspaces are preserved intact with a visible notice and **Export
previous workspace**; their pending plans/evidence cannot approve a new bundle.
Reset retains that legacy export and the model cache.

## Verification and publication

```sh
node --test tests/*.test.mjs
python3 tests/page-workspace-browser.py
python3 tests/page-recovery-browser.py
python3 tests/page-storage-browser.py
python3 tests/page-check-semantics-browser.py
python3 tests/page-check-isolation-browser.py
node scripts/generate-workboard-preview.mjs --check
python3 scripts/check-public.py
```

The full-page browser suite uses a labeled synthetic model adapter and the actual
production DOM runtime. It is NON_ACCEPTANCE evidence for Gemma inference. Old
policy/appearance module tests remain compatibility checks; their historical UI
browser harnesses do not validate the new full-page UI. Before delivery, run real
Gemma in a desktop browser: add the date column, add a separate interactive feature,
inspect actual file edits/check results, accept, reload and review Recorded.
Repeat at the hosted URL to verify deployment asset paths and origin-local cache.

Pages uploads only `demo/`. This does not publish a user's generated app, change
the Rust runtime, or add CI requirements. The SDK/model/WASM versions remain
pinned; the default web model is pinned at commit
`e6a3892a8c22cf87ff99583996149cf3cb32452b`. The browser engine context is
8192 tokens. Coder's first attempt disables extended reasoning and uses up to
2048 output tokens; its two repair attempts enable reasoning with up to 4096.
Planner and Tester use up to 2048, with temperature 0.1. Coder's constrained SDK function
declarations return patch/file data only: no tool is executed. Plain-text JSON
remains a compatibility path, and all proposals pass the same source/browser
gates. The previous E2B cache entry is retained under its original URL for
rollback; the E4B download requires about 3.3 GB of free browser storage. No model
cross-device speed or success rate is claimed.

`tests/browser-model-benchmark.html` is a manual real-model comparison page,
excluded from the Pages artifact. It freezes an actual approved plan and base,
compares identical Coder prompt hashes, retains failed attempts and exports
per-role timing/token counters and fresh browser evidence. On one Mac Chrome
comparison, the date request completed build plus QA in 39.4 seconds versus
74.6 seconds with the prior reasoning settings; both passed 5/5 and were
explicitly accepted. These are individual device observations, not a latency
guarantee. `tests/page-runtime-performance-browser.py` separately measures DOM
check throughput with synthetic fixtures; those results do not measure Gemma.

## Official developer workboard preview

The topbar’s **Developer workboard** link opens `./workboard/`, a separate public
screen generated from the real application’s `public/index.html`, `styles.css`,
`workboard.js` and `workboard-model.js`. The official layout, cards, stage controls,
filters, repository queues, detail sections, Attention, Main and System views are
reused. It is a read-only preview with clearly labeled fictional tasks and a
frozen sample snapshot. No real account, repository, worker or private session is
connected. Agent statuses illustrate the sample snapshot; no agents run here.

The explicit `workboard/preview-data.mjs` adapter serves only an allowlist of local
GET reads. It rejects writes and unknown routes without contacting a backend.
Task creation, planning, editing, priority changes, launches and delivery remain
disabled. A `connect-src 'none'` policy prevents network transports. This screen
neither loads Gemma nor shares the expense demo’s live model session.

Update the official UI first, then regenerate and check the committed preview:

```sh
node scripts/generate-workboard-preview.mjs
node scripts/generate-workboard-preview.mjs --check
node --test tests/workboard-preview.test.mjs
python3 tests/workboard-preview-browser.py
```

The generator asserts every adaptation anchor, copies the official stylesheet and
model module byte-for-byte, records source hashes, and changes only the copied
HTML’s relative paths/preview framing and the copied JS’s explicit preview
transport, labels, read navigation, polling and storage namespace. `--check`
rejects source drift or hand-edited generated outputs. Keep the adapter and
`preview.css` as the small handwritten preview layer; edit the generator rather
than generated files. No production `public/` or Rust runtime changes are needed.
The existing Pages workflow uploads only `demo/`; regenerating the committed
preview changes that directory and triggers publication. Run the drift check
locally whenever official assets change; this adds no CI requirement.

## References

- [LiteRT-LM browser API](https://developers.google.com/edge/litert-lm/js)
- [Gemma 4 E4B artifact and model terms](https://huggingface.co/litert-community/gemma-4-E4B-it-litert-lm)
- [Pages custom workflows](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages)
