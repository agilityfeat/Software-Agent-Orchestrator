# Agents at Work — browser demo

A separate, static workshop app: a builder workboard next to **V135 Spend**, a
fictional expense inbox. It does not replace, connect to, or publish the Rust
orchestrator's private runtime. No API key or server is required.

## Run

From the repository root:

```sh
python3 -m http.server 8080
# Open http://localhost:8080/demo/
node --test tests/demo.test.mjs
```

Use a current desktop browser with WebGPU for **Live local agents**. Click
**Load Gemma** to explicitly download approximately 2 GB, or select an existing
compatible `.litertlm` file. Cache storage needs sufficient free space and may
be evicted by the browser. Reset demo preserves the model cache. The package,
WASM and model hosts need network access on first use; this is not an offline
installation guarantee. Model weights are not distributed in this repository.

Start with **Guided example** to explore without downloading a model. Its agent
outputs are explicitly scripted, but its tests execute the candidate function.
The first candidate deliberately gets the exact threshold wrong; the second
fixes it. This is not represented as a recorded model execution.

## Workshop flow

1. Describe the expense policy in one prompt. For example: “Require manager approval for expenses above $500. Preserve already paid expenses.” Add “Require receipts above $75” only when wanted.
2. Create a plan. Review **What the change must do**, the interpreted rules alongside the plan, before approving it.
3. Approve the plan and run Planner → Coder → Tester with separate role contexts.
4. Inspect the candidate source, real test results, hash and repair history.
5. Compare Accepted/Candidate previews, then **Accept change** to promote locally.

The model performs planning before authorization; the approved execution starts
with Coder. It is one shared model, not three simultaneously loaded models.
Two correction cycles are permitted after the initial implementation.

Only `evaluateExpense(expense)` is editable. The money contract uses integer
cents. Approval applies strictly **above** the threshold, paid history remains
unchanged, and a missing required receipt takes priority over manager review.
The prompt supplies the amounts. A bounded, deterministic reader supports manager
approval and optional receipt requirements strictly above a dollar amount,
including “Expenses above $500 need manager review.” Missing amounts, conflicting
limits, negation, ranges, inclusive limits and unsupported conditions are rejected
before planning; there is no hidden default amount. Editing the prompt clears the
previous plan so it cannot authorize an older request. Saved pending plans must
match their request before they can proceed. Models never choose the expected
outcomes of the protected tests.
This initial slice does not implement arbitrary component/repository editing.

The workboard uses Backlog → Ready → Active → Review → Done with an explicit
Blocked condition. This demo handles one current task, not a multi-project queue.
Accepted code and recent runs persist in IndexedDB. Active work is marked
interrupted on reload instead of silently restarting. Recorded mode is read-only
and is available only after an actual Live run has been saved on that browser.
Exports contain local source and evidence, not a GitHub commit. Expense approval
buttons and receipts are synthetic; no payment or external business action occurs.

## Implementation

- No-build HTML/CSS/ES modules keep the demo independent of the Rust starter.
  This deliberately avoids introducing React/Vite or a Node build requirement.
- `model-worker.mjs`: one LiteRT-LM 0.18.0 engine in a classic worker, with
  an explicit pinned CDN locator for its WASM assets, fresh conversations per role,
  sequential inference, download progress and a versioned model cache. JS and
  WASM versions match; 0.18.1 is intentionally avoided because of its reported
  browser asset regression. Generation and load operations have deadlines;
  cancellation terminates the model worker.
- `sandbox-worker.mjs`: bundled QuickJS/WASM 0.31.0, new isolated context per input,
  16 MB memory limit, 256 KB stack limit and interrupt deadline. Generated code
  receives JSON data only, with no host DOM, network, credentials or filesystem
  API. An outer worker timeout is an additional limit. Pinned modules, embedded
  WASM, licenses and upstream SHA-256 provenance live in `vendor/quickjs-0.31.0/`;
  candidate execution requires no CDN imports or broader worker policy.
- `prompt-policy.mjs`: explicit prompt-to-policy interpretation; unsupported or
  ambiguous requests produce a plain clarification instead of an inferred amount.
- `core.mjs`: protected expectations, output schemas and exact-code SHA-256.
  The Tester proposes data-only cases; the host calculates expected results.
  A model's prose never creates a passing test. Default-policy checks include
  missing-receipt boundaries; preview and acceptance recheck every displayed
  expense against the current oracle, including saved runs with old evidence.
  Using the same model for QA does not provide independent model judgment; deterministic checks are essential.
- `controller.mjs`: authorization, bounded repair, versioned evidence, cancellation
  fencing and exact-result acceptance. Previously tested code cannot be edited
  and accepted under an old hash.
- `agent-activity.mjs`: Planning, Building and Checking icons show which role
  is working from the actual controller state, with text and reduced-motion support.
- `app.mjs`: resizable side-by-side/top-bottom views, responsive expense inbox,
  accepted/candidate source comparison, search, filters, evidence and local history.

No application telemetry, cloud inference fallback or GitHub credentials are
included. Third-party download hosts receive ordinary asset requests. Use only
synthetic data: this is a workshop prototype, not financial/security software.

## Publish with GitHub Pages

The included **Publish browser demo to Pages** workflow is deployment-only; it
adds no CI requirement to the existing Rust project. It uploads **only `demo/`**,
not the repository, tests or runtime files. It deploys after a relevant push to
`main`, or a manual workflow dispatch on `main`. Pull requests do not deploy.

1. In repository **Settings → Pages**, select **GitHub Actions** as the source.
2. Merge the demo PR; review the Pages deployment job and its returned site URL.
3. Open that URL and validate both a cold load and a cached load on the presenter
   laptop. Resolve any environment approval requested by GitHub.

All asset/worker paths are relative, so the project site works below a repository
subpath as well as at a domain root. The workflow does not change repository
visibility or configure Pages settings. Creating this PR does not make the site
live. To disable publishing, disable the deployment workflow/Pages in settings.

## Validation and limitations

```sh
# Deterministic contract/controller tests; no model or paid provider calls.
node --test tests/demo.test.mjs

# Browser smoke: install Playwright separately, outside the repository runtime.
python3 -m pip install playwright
python3 -m playwright install chromium
python3 tests/demo-browser.py

# Optional UI-only adapter when external sandbox assets are unavailable.
python3 tests/demo-browser.py --offline-ui
```

The normal browser smoke exercises bundled real QuickJS workers with external
HTTPS requests blocked, without downloading Gemma. `--offline-ui` replaces ONLY
the test's sandbox route with a Node VM adapter; it does not validate the production sandbox. Screenshots go to a
system temporary directory by default, outside the checkout.

Deterministic tests and the production QuickJS browser smoke pass, including
host-API isolation, interruption, acceptance, persistence and rejection of stale
passing evidence. On 2026-10-25, a native desktop Chrome Live run downloaded the
2.01 GB Gemma artifact, initialized on WebGPU, completed Planner → Coder → Tester,
and passed 10 executed checks before human acceptance. The $500 expense remained
eligible, larger submitted expenses required manager review, and paid history
remained paid. Reloading reused the browser model cache; Recorded mode retained
the accepted real run. This is one laptop's observed result, not a performance
or cross-device success guarantee. Repeat the Live smoke at the Pages deployment
URL to verify hosted asset paths and that browser's origin-specific cache.

Before presenting, run the normal browser smoke and then a real model task on the
presenter and a second laptop. Check GPU compatibility, cold/cached load, planner
JSON, coder output, Tester cases, cancellation and actual latency. Save a successful
Live run to enable honest Recorded replay. No model speed or success rate is claimed.

## References

- [LiteRT-LM browser API](https://developers.google.com/edge/litert-lm/js)
- [Pinned LiteRT-LM browser package](https://github.com/google-ai-edge/LiteRT-LM/tree/v0.18.0/js/packages/core)
- [Browser release regression](https://github.com/google-ai-edge/LiteRT-LM/issues/4051)
- [Gemma 4 E2B artifact and model terms](https://huggingface.co/litert-community/gemma-4-E2B-it-litert-lm)
- [QuickJS WASM](https://github.com/justjake/quickjs-emscripten)
- [Pages custom workflows](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages)
