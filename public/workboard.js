import { repositoryFreeResearch, kanbanMovePayload, cardPriorityPayload, sortQueuedTasks, liveRunStates, repositoryQueueStatus, codingAgentSelection, defaultQaPolicy, dependencyReason, planSelection, coalesceActivity, qaBugLabel, initialAutomationPolicy, readyTaskPayload, automationLabel, isArchived, columns, labels, priorities, activeRun, taskStatus, attentionReason, filterTasks, safeWebUrl, safeArtifactUrl, taskPayload, activityTitle, executionNotice, privateWorkspaceLink, descriptionParts, safeGithubTaskUrl, latestRun, taskBlocker, activityTime, executionReason } from "./workboard-model.js";

const $ = selector => document.querySelector(selector);
const api = "/api/workboard/v1";
function rememberedStage() {
  try { const stage = localStorage.getItem("workboard-stage"); if (columns.includes(stage)) return stage; } catch { /* Stage selection also works without browser storage. */ }
  return "backlog";
}
const state = { authenticated: false, connected: false, csrf: null, capabilities: {}, execution: null, codingAgents: null, plans: [], projects: [], tasks: [], runs: [], events: [], watermark: 0, selected: null, stage: rememberedStage(), editing: null, busy: false, loaded: false, view: "work", lastUpdate: null, authGeneration: 0, executionActivity: {state:"unavailable",tasks:{}} };
const planEdits=new Map();
let editorQaExplicit=false;
let agentDraft=null;
let draggedTask=null;
let stream = null, refreshPromise = null, toastTimer = null, detailRequest = 0, eventRefresh = null, capabilityRequest = 0, capabilityApplied = 0;
const pendingCommands = new Map();

function node(tag, text, className) {
  const item = document.createElement(tag);
  if (text != null) item.textContent = String(text);
  if (className) item.className = className;
  return item;
}
function linkedText(tag, text, className) {
  const item = node(tag, null, className);
  for (const part of descriptionParts(text)) {
    if (!part.href) { item.append(document.createTextNode(part.text)); continue; }
    const link = node("a", part.text, "description-link");
    link.href = part.href; link.target = "_blank"; link.rel = "noopener noreferrer";
    item.append(link);
  }
  return item;
}
function displayTime(value) {
  const date = new Date(value);
  return !value || !Number.isFinite(date.getTime()) ? "Time unavailable" : new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(date);
}
function displayHistoricalTime(value) {
  const date = new Date(value);
  return !value || !Number.isFinite(date.getTime()) ? "Time unavailable" : new Intl.DateTimeFormat(undefined, { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(date);
}
function readable(value) { return String(value || "unknown").replace(/[._]/g, " "); }
function shortId(value) { return String(value || "Task").replace(/^task[_-]/, "").slice(0, 12).toUpperCase(); }
function array(value) { return Array.isArray(value) ? value : []; }
function sortedRuns(value) { return array(value).slice().sort((a, b) => String(b.created_at || "").localeCompare(String(a.created_at || ""))); }
function allowed(capability = "write") { return state.authenticated && state.connected && !state.busy && state.capabilities[capability] === true; }
function notify(message) {
  clearTimeout(toastTimer); $("#toast").textContent = message; $("#toast").hidden = false;
  toastTimer = setTimeout(() => { $("#toast").hidden = true; }, 4500);
}
function errorMessage(error) {
  const messages = { research_result_invalid:"The research report, sources or review could not be verified. Reconcile this run before trying again.",research_unavailable:"Repository-free research needs the native agent runtime.",research_repository_required:"Without a repository, select Research proposals only.",research_qa_exhausted:"The report still has QA findings after its repair limit.",research_repair_queued:"QA requested a report repair.", agent_unavailable:"Enable or replace the assigned agent in System before starting new work.", native_setup_blocked:"The coding agent could not finish project setup. Check the recorded setup failure before retrying.",native_validation_incomplete:"The agent did not complete the required project checks. This task has not passed validation.",native_validation_invalid:"The reported checks could not be matched to completed commands. Review the agent result before retrying.",native_auth_refresh_unknown:"Provider sign-in changed during the run. Reconcile access before continuing this repository.",native_cleanup_unknown:"Worker cleanup has not been confirmed. Reconcile this run before continuing the repository.",native_deadline_exceeded:"The coding-agent stage reached its deadline.",native_cancelled:"The coding-agent stage stopped before completion.",native_process_failed:"The coding agent exited before completion. Check its recorded failure before retrying.",native_result_invalid:"The agent returned a result that could not be verified.",native_turn_incomplete:"The coding agent did not finish its turn.",native_qa_source_changed:"QA changed source files. A clean independent review is required before delivery.",native_protected_source_path:"The implementation changed protected project metadata. Review those changes before continuing.",native_unignored_environment:"Setup files are included in the source changes. Keep local environments out of the implementation.", coding_agent_unavailable:"Choose an available coding agent before saving or starting this task.", invalid_coding_agent:"Choose a registered coding agent.", task_too_large:"Shorten the objective, context or acceptance criteria so the complete task fits its worker request.", body_too_large:"Shorten the objective, context or acceptance criteria before saving this request.", invalid_task_type:"Choose a supported task type.", invalid_labels:"Use up to 12 short labels with letters, numbers, hyphens or slashes.", invalid_dependencies:"Select each prerequisite task once.", dependency_not_found:"A prerequisite no longer exists. Update the dependency selection.", dependency_cycle:"Dependencies cannot refer back to this task.", dependencies_unmet:"Finish the prerequisite tasks before starting this task.", planning_dependency_unselected:"Select the prerequisite proposals too, or remove those dependencies.", planning_not_reviewable:"This plan is no longer awaiting review. Refresh its latest state.", planning_input_invalid:"Shorten the goal or context, and enter a project label plus owner/repository.", planning_busy:"Finish or discard the current plan before generating another.", planning_unavailable:"AI planning is not configured on this host.", planning_selection_invalid:"Select each proposed task once and check its edited fields.", planning_result_invalid:"The generated plan could not be verified. Discard it before trying another.", plan_not_reviewable:"This plan is no longer awaiting review. Refresh its latest state.", plan_already_accepted:"This plan has already added its selected tasks to Backlog.", planning_cleanup_unknown:"Planning cleanup could not be confirmed. Check cleanup before generating another.", project_profile_sync_required:"The local package or build settings differ from GitHub’s default branch. Sync those settings before connecting; your local folder stays unchanged.", project_remote_changed:"The repository or default branch changed. Reconnect with a freshly reviewed project configuration.", project_setup_unavailable:"Project setup is not configured on this host.", project_profile_unsupported:"This repository does not match a supported Node test/build recipe.", project_source_dirty:"Commit or set aside your local changes before connecting this folder.", project_already_exists:"This folder or repository is already connected.", project_private_repository_required:"This version supports private GitHub repositories. Choose a private repository.", project_github_permission_required:"The configured GitHub account needs push access to this repository.", project_filter_unsupported:"This checkout uses a Git clean filter. Connect a clean checkout without executable filters.", project_github_check_failed:"GitHub access could not be verified. Check the host connection and try again.", project_needs_reconciliation:"Check the interrupted task’s outcome before restarting this project.", project_paused:"This repository is paused. Resume its queue to continue.", project_clone_failed:"The default branch could not be cloned with the configured GitHub account.", project_cache_build_failed:"Offline dependencies could not be prepared. Check the host Docker and npm registry connection.", adapter_profile_invalid:"The repository has unsupported dependency sources or check settings.", project_preparing:"Repository preparation is already running.", project_needs_preparation:"Wait for this task’s repository checks to finish preparing, or choose Prepare checks again.", project_discovery_interrupted:"Check preparation was interrupted. Prepare checks again.", automation_unavailable: "Automatic pickup is not activated for this owner. No enrollment was saved.", automation_scope_mismatch: "This activation applies to another repository.", automatic_run_already_consumed: "This automatic activation has already been used. Review its recorded run.", execution_ownership_unavailable: "Execution ownership is unavailable or changed. No work was launched.", invalid_repository: "Use owner/repository without a URL or .git suffix.", invalid_task_text: "Check the task text and field length limits.", invalid_github_url: "Use a GitHub issue or PR URL matching this repository.", execution_busy: "Another task is active for this project or repository.", reconciliation_required: "An interrupted action needs its outcome checked before execution can continue.", workflow_locked: "Engineering tasks must keep their engineering workflow.", revision_conflict: "This task changed. Refresh and review its latest revision before trying again.", stale_revision: "This task changed. Refresh and review its latest revision before trying again.", unauthorized: "Your session has ended. Reconnect to continue.", csrf_rejected: "Your session changed. Reconnect and try again.", csrf_invalid: "Your session changed. Reconnect and try again.", execution_unavailable: "The execution host is unavailable. No run was started.", execution_paused: "Execution is paused. No run was started.", active_run_exists: "Another task is active for this project or repository.", task_has_active_run: "An active run owns this task. Stop and reconcile it before editing.", forbidden: "This action is not available for this operator." };
  return messages[error.code] || (error.network ? "Connection lost. Check the task history before repeating this action." : "The action was not accepted. Refresh and check the task’s activity before trying again.");
}
async function request(path, { method = "GET", body, key, timeoutMs } = {}) {
  const authGeneration = state.authGeneration;
  const headers = { Accept: "application/json" };
  const command = path === "/session" ? null : `${method} ${path} ${JSON.stringify(body)}`;
  if (body !== undefined) headers["Content-Type"] = "application/json";
  if (method !== "GET") {
    if (state.csrf) headers["X-CSRF-Token"] = state.csrf;
    // Session login/logout have no idempotency receipt; never retain their bodies.
    if (command !== null) {
      headers["Idempotency-Key"] = key || pendingCommands.get(command) || crypto.randomUUID();
      pendingCommands.set(command, headers["Idempotency-Key"]);
    }
  }
  let response;
  try { response = await fetch(`${api}${path}`, { method, headers, credentials: "same-origin", cache: "no-store", ...(timeoutMs ? { signal: AbortSignal.timeout(timeoutMs) } : {}), body: body === undefined ? undefined : JSON.stringify(body) }); }
  catch { if (authGeneration === state.authGeneration) setConnection(false); throw Object.assign(new Error("Network unavailable"), { network: true }); }
  let data = {};
  try { data = await response.json(); } catch { /* A finite error code below is enough for display. */ }
  if (!response.ok) {
    pendingCommands.delete(command);
    if (response.status === 401 && path !== "/session" && authGeneration === state.authGeneration) expireSession();
    throw Object.assign(new Error("Request rejected"), { code: data.error || `http_${response.status}`, status: response.status });
  }
  pendingCommands.delete(command);
  return data;
}
function setConnection(connected) {
  state.connected = connected;
  const indicator = $("#board-connection");
  indicator.className = `connection ${!state.authenticated ? "connecting" : connected ? "online" : "offline"}`;
  indicator.lastElementChild.textContent = !state.authenticated ? "Access required" : connected ? "Connected" : "Disconnected";
  updateControls();
}
function updateControls() {
  $("#new-task").disabled = !allowed(); $("#empty-create").disabled = !allowed();
  $("#save-task").disabled = !allowed();
  document.querySelectorAll("[data-command]").forEach(button => {
    const capability = button.dataset.capability || "write";
    button.disabled = !allowed(capability) || button.dataset.unavailable === "true";
  });
  const execution = state.execution;
  $("#execution-controls").hidden = !state.authenticated || !execution?.approved;
  $("#execution-pause").disabled = !allowed();
  $("#execution-pause").textContent = execution?.paused ? "Resume launches" : "Pause launches";
  $("#execution-state").textContent = execution?.paused ? "All new launches paused." : state.projects.length ? "Repositories work in parallel · one active task per repository." : execution?.lane_run_id ? "One task owns the execution lane." : "Ready for one task at a time.";
  const notice = executionNotice(state.capabilities, state.connected);
  $("#board-notice").hidden = !state.authenticated || !notice;
  $("#board-notice").textContent = notice || "";
  document.querySelectorAll("[data-execution-notice]").forEach(item => {
    item.hidden = state.connected && state.capabilities.run === true;
    item.textContent = notice || "Agent execution is unavailable. This task is saved; a run has not been started.";
  });
}
function expireSession() {
  state.authGeneration++;
  state.authenticated = false; state.csrf = null; state.capabilities = {}; state.execution = null; state.codingAgents = null;
  if (stream) { stream.close(); stream = null; }
  $("#access-panel").hidden = false; $("#sign-out").hidden = true;
  $("#operator-name").textContent = "Private access required";
  state.system=null;agentDraft=null;if($("#agents-editor").open)$("#agents-editor").close();$("#agent-fields").replaceChildren();$("#agent-defaults").replaceChildren();
  draggedTask=null;state.plans=[];planEdits.clear();pendingCommands.clear();state.projects = []; state.tasks = []; state.runs = []; state.events = []; state.executionActivity = {state:"unavailable",tasks:{}}; state.loaded = false;
  $("#main-goal-form").reset(); $("#main-error").textContent = "";
  $("#task-form").reset(); $("#task-dependencies").replaceChildren(); $("#task-coding-agent").replaceChildren();
  $("#editor-error").textContent = ""; $("#detail-content").replaceChildren();
  state.editing = null; state.selected = null;
  if ($("#task-detail").open) $("#task-detail").close();
  if ($("#task-editor").open) $("#task-editor").close();
  $("#task-board").replaceChildren(); $("#attention-list").replaceChildren();
  render();
  setConnection(false);
}
function applySession(session) {
  state.authenticated = session.authenticated === true;
  state.csrf = session.csrf_token || null;
  state.capabilities = session.capabilities || {};
  $("#access-panel").hidden = state.authenticated;
  $("#sign-out").hidden = !state.authenticated || session.login_method === "tailnet";
  $("#login-form").hidden = session.login_method === "tailnet";
  const tailnet = session.login_method === "tailnet";
  $("#access-message").textContent = tailnet ? "Connect through your private owner account. This route does not accept operator keys." : "Sign in once with the local operator key. This browser remembers access until you sign out or the key changes.";
  const privateLink = $("#private-workspace-link");
  const privateUrl = privateWorkspaceLink(session.private_workspace_url);
  privateLink.hidden = !privateUrl || tailnet;
  if (privateUrl) privateLink.href = privateUrl; else privateLink.removeAttribute("href");
  $("#private-workspace-hint").hidden = privateLink.hidden;
  const identity = typeof session.identity === "string" ? session.identity : session.identity?.display_name || session.identity?.login || session.identity?.email;
  $("#operator-name").textContent = identity || (state.authenticated ? "Owner session" : "Private access required");
  if (!state.authenticated) { render(); setConnection(false); }
  updateControls();
}
async function bootstrap() {
  try {
    applySession(await request("/session"));
    if (state.authenticated) { await refresh(); connectEvents(); }
  } catch (error) { expireSession(); $("#access-message").textContent = error.network ? "The execution host cannot be reached. Reconnect to your private network and refresh." : "Private access is unavailable. Check the host configuration or use the local operator key."; }
}
async function checkRememberedSession() {
  if (!state.authenticated) return;
  const generation = state.authGeneration;
  try {
    const session = await request("/session");
    if (generation !== state.authGeneration || !state.authenticated) return;
    if (session.authenticated !== true) {
      expireSession(); notify("Session ended. Reconnect to continue.");
    } else applySession(session);
  } catch { /* A transient restart/network failure keeps drafts and the existing EventSource retry. */ }
}
function connectEvents() {
  if (!state.authenticated || stream) return;
  stream = new EventSource(`${api}/events?after=${encodeURIComponent(state.watermark)}`);
  stream.onopen = () => { if (state.authenticated) { setConnection(true); void refresh(); } };
  stream.onerror = () => { setConnection(false); void checkRememberedSession(); };
  stream.addEventListener("workboard", event => {
    try { const record = JSON.parse(event.data); state.watermark = record.cursor ?? state.watermark; } catch { /* A fresh snapshot resolves malformed events. */ }
    clearTimeout(eventRefresh); eventRefresh = setTimeout(() => void refresh(), 120);
  });
  stream.addEventListener("session_expired", () => { expireSession(); notify("Session ended. Reconnect to continue."); });
}
async function refresh() {
  if (!state.authenticated) return;
  if (refreshPromise) return refreshPromise;
  const authGeneration = state.authGeneration, capabilityOrder = ++capabilityRequest;
  refreshPromise = (async () => {
    try {
      const data = await request("/snapshot", { timeoutMs: 5000 });
      if (!state.authenticated || authGeneration !== state.authGeneration) return;
      state.system = data.system || null; state.codingAgents = data.coding_agents || null; state.execution = data.execution || null; state.executionActivity = data.execution_activity || {state:"unavailable",tasks:{}}; state.plans=array(data.plans); state.projects = array(data.projects); state.maintenance = data.maintenance; state.tasks = array(data.tasks); state.runs = sortedRuns(data.runs); state.events = array(data.events);
      state.watermark = data.watermark ?? data.cursor ?? state.watermark;
      applyCapabilities(data.capabilities, capabilityOrder);
      state.loaded = true; state.lastUpdate = new Date().toISOString(); setConnection(true); render();
      if (state.selected && $("#task-detail").open) await loadDetail(state.selected, false);
    } catch (error) { if (!state.authenticated || authGeneration !== state.authGeneration) return; setConnection(false); if (!state.loaded) $("#task-total").textContent = state.authenticated ? "Workspace unavailable" : "Private access required"; }
    finally { refreshPromise = null; }
  })();
  return refreshPromise;
}
function applyCapabilities(capabilities, order) {
  if (capabilities && order >= capabilityApplied) {
    state.capabilities = capabilities; capabilityApplied = order;
  }
}
function populateFilter(selector, values, defaultLabel) {
  const select = $(selector), value = select.value;
  select.replaceChildren(new Option(defaultLabel, ""));
  [...new Set(values.filter(Boolean))].sort().forEach(item => select.append(new Option(item, item)));
  if ([...select.options].some(option => option.value === value)) select.value = value;
}
function updateFilterControls() {
  const count=state.authenticated ? [$("#archive-filter").value!=="active",...["#project-filter","#owner-filter","#type-filter","#label-filter"].map(selector=>Boolean($(selector).value))].filter(Boolean).length : 0;
  $("#filter-count").textContent=count ? String(count) : "";$("#filter-count").hidden=count===0;
  $("#reset-filters").disabled=count===0;
  if(!state.authenticated)$("#board-filter-menu").open=false;
}
function resetBoardFilters() {
  $("#archive-filter").value="active";
  for(const selector of ["#project-filter","#owner-filter","#type-filter","#label-filter"])$(selector).value="";
  render();
}

function render() {
  renderSystem();renderMain();renderProjects();
  if(["main","system"].includes(state.view))$("#projects-panel").hidden=true;
  $(".board-toolbar").hidden = !state.authenticated;
  $("#board-stages").hidden = !state.authenticated;
  $(".board-scroll-hint").hidden = !state.authenticated;
  if (!state.authenticated) {
    updateFilterControls();
    $("#task-board").replaceChildren(); $("#task-board").hidden = true;
    $("#board-stages").replaceChildren();
    $("#attention-list").replaceChildren();
    $("#board-empty").hidden = true; $("#attention-empty").hidden = true;
    $("#work-count").textContent = "—"; $("#attention-count").textContent = "—";
    $("#task-total").textContent = "Sign in to load tasks";
    $("#archive-status").textContent = "";
    $("#board-freshness").textContent = "Sign in to load workspace data";
    updateControls(); return;
  }
  if (draggedTask) return; // Keep native drag targets mounted during live refresh.
  populateFilter("#project-filter", state.tasks.map(task => task.project), "All projects");
  populateFilter("#owner-filter", state.tasks.map(task => task.owner), "All owners");
  populateFilter("#label-filter",state.tasks.flatMap(task=>array(task.labels)),"All labels");
  updateFilterControls();
  const tasks = filterTasks(state.tasks, { task_type:$("#type-filter").value,label:$("#label-filter").value,search: $("#task-search").value, project: $("#project-filter").value, owner: $("#owner-filter").value, scope: $("#archive-filter").value, runs:state.runs });
  $("#work-count").textContent = String(state.tasks.filter(task => !isArchived(task,state.runs) && !["done","cancelled"].includes(task.status)).length);
  $("#task-total").textContent = `${tasks.length} ${tasks.length === 1 ? "task" : "tasks"}`;
  $("#archive-status").textContent = "Archive completed tasks from their detail view.";
  const board = $("#task-board"); board.replaceChildren();
  const grouped = new Map(columns.map(status => [status, tasks.filter(task => taskStatus(task, state.runs) === status)]));
  const stages = $("#board-stages");
  // Keep buttons mounted so polling and filter changes preserve keyboard focus.
  if (!stages.childElementCount) {
    for (const status of columns) {
      const button = node("button", null, "stage-button");
      button.type = "button"; button.dataset.stage = status;
      button.setAttribute("aria-controls", "task-board"); wireKanbanDrop(button, status);
      button.append(node("span", labels[status]), node("span", "0", "stage-count"));
      stages.append(button);
    }
  }
  for (const button of stages.children) {
    const status = button.dataset.stage, count = grouped.get(status).length;
    button.setAttribute("aria-pressed", String(state.stage === status));
    button.setAttribute("aria-label", `${labels[status]}, ${count} ${count === 1 ? "task" : "tasks"}`);
    button.lastElementChild.textContent = String(count);
  }
  const visibleColumns = columns.filter(status => status !== "cancelled" || grouped.get(status).length || state.stage === status);
  board.style.setProperty("--board-columns", visibleColumns.length);
  for (const status of visibleColumns) {
    const section = node("section", null, "board-column"), heading = node("h2", null, "column-heading");
    section.id = `stage-${status}`; section.dataset.selected = String(state.stage === status); section.dataset.stage = status;
    wireKanbanDrop(section, status);
    heading.id = `stage-heading-${status}`; section.setAttribute("aria-labelledby", heading.id);
    const rows = ["backlog", "ready"].includes(status) ? sortQueuedTasks(grouped.get(status)) : grouped.get(status);
    heading.append(node("span", null, `column-dot ${status}`), node("span", labels[status]), node("span", rows.length, "column-count"));
    const list = node("div", null, "task-column-list");
    rows.forEach(task => list.append(taskCard(task)));
    if (!rows.length) {
      const empty = node("div", null, "column-empty");
      empty.append(node("span", `No ${labels[status].toLowerCase()} tasks`), node("span", "Choose another stage or adjust your filters.", "lane-empty-hint"));
      list.append(empty);
    }
    section.append(heading, list); board.append(section);
  }
  const archiveMode=$("#archive-filter").value==="archive";$("#board-empty h2").textContent=archiveMode?"No archived tasks":state.tasks.length?"No matching tasks":"Your next outcome starts here";$("#empty-create").hidden=archiveMode||state.tasks.length>0;
  $("#board-empty").hidden = !state.loaded || tasks.length > 0;
  board.hidden = state.loaded && tasks.length === 0;
  $(".board-scroll-hint").hidden = board.hidden;
  const attention = state.tasks.map(task => ({ task, reason: attentionReason(task, state.runs) })).filter(item => item.reason);
  $("#attention-count").textContent = String(attention.length);
  $("#attention-list").replaceChildren(...attention.map(({ task, reason }) => {
    const button = node("button", null, "attention-card"), content = node("div", null, "attention-content");
    const review = task.status === "review" && !task.blocked_reason;
    button.append(node("span", review ? "✓" : "!", `attention-indicator ${review ? "" : "blocked"}`));
    content.append(node("div", task.title, "task-title"), node("p", `${reason} · ${task.project || "No project"}`, "meta"));
    button.append(content, node("span", "→", "attention-arrow")); button.addEventListener("click", () => void openDetail(task.id)); return button;
  }));
  $("#attention-empty").hidden = !state.loaded || attention.length > 0;
  $("#board-freshness").textContent = state.lastUpdate ? `Updated ${displayTime(state.lastUpdate)}${state.connected ? "" : " · stale"}` : "Waiting for workspace data";
  updateControls();
}
function executionSummary(task, runs, activity, detailed = false) {
  const block = node("div", null, `execution-summary${detailed ? " detailed" : ""}`), latest = latestRun(task, runs), roles = activity?.tasks?.[task.id];
  function line(label, status, at, hint) {
    const row = node("div", null, "execution-line"), time = activityTime(at);
    row.append(node("span", label, "execution-label"), node("span", status, "execution-status"));
    if (time) { const item = node("time", time.relative); item.dateTime = time.iso; item.title = `${hint}: ${time.exact}`; row.append(item); if (detailed) row.append(node("span", time.exact, "execution-exact")); }
    else if (at) row.append(node("span", "Time unavailable"));
    block.append(row);
  }
  if (latest) {
    line("Last run requested", readable(latest.status), latest.created_at, "Run requested");
    if (latest.stage) line("Current stage", readable(latest.stage), latest.updated_at, "Stage recorded");
  }
  else block.append(node("p", roles?.coder || roles?.qa ? "Retained execution history" : activity?.state === "available" ? task.source_id || task.source_url || task.source_history ? "No verified retained execution" : "No execution recorded" : "Execution history unavailable", "execution-empty"));
  for (const [role,label] of [["coder","Coder"],["qa","QA"]]) {
    const fact = roles?.[role], completed = fact?.basis === "worker_completion", recorded = latest?.evidence?.[role === "coder" ? "coder_status" : "qa_status"];
    const status = recorded && !["not_started", "unavailable"].includes(recorded) ? ({complete:"Complete",passed:"Passed",failed:"Failed"})[recorded] || readable(recorded) : fact ? `${({running:"Running",delivery_pending:"Delivery pending",complete:"Complete",failed:"Failed",blocked:"Blocked",cancelled:"Canceled"})[fact.status] || "Recorded"}${fact.provenance === "retained_linear" ? " · retained" : ""}` : activity?.state === "available" ? "—" : "— unavailable";
    line(label, status, fact?.at, completed ? `${label} terminal receipt recorded` : `${label} receipt first observed in this state`);
  }
  return block;
}
function clearKanbanDrag() {
  draggedTask = null;
  document.querySelectorAll(".board-column, .stage-button").forEach(target => { delete target.dataset.dropAllowed; target.classList.remove("drop-over"); });
}
function wireKanbanDrop(target, status) {
  target.addEventListener("dragover", event => {
    if (!draggedTask || !allowed() || !kanbanMovePayload(draggedTask, status, state.runs)) return;
    event.preventDefault(); event.dataTransfer.dropEffect = "move"; target.classList.add("drop-over");
  });
  target.addEventListener("dragleave", event => { if (!target.contains(event.relatedTarget)) target.classList.remove("drop-over"); });
  target.addEventListener("drop", event => {
    const task = draggedTask, payload = task && kanbanMovePayload(task, status, state.runs);
    if (!payload || !allowed()) return;
    event.preventDefault(); clearKanbanDrag();
    void mutate(`/tasks/${encodeURIComponent(task.id)}`, "PATCH", payload, status === "ready" && task.workflow === "engineering" ? "Ticket queued for automatic processing." : `Moved to ${labels[status]}.`).then(result => { if (result) { state.stage = status; render(); } });
  });
}
function taskCard(task) {
  const card = node("article", null, "task-card"), button = node("button", null, "task-card-open"), head = node("div", null, "task-card-head");
  button.type = "button"; button.setAttribute("aria-label", `View ${task.title || "Untitled task"}`);
  const running = activeRun(task, state.runs), locked = Boolean(running || task.archived_at || ["done", "cancelled"].includes(task.status));
  const grip = node("span", "⠿", "task-drag-handle");
  grip.setAttribute("aria-hidden", "true");
  grip.draggable = !locked && allowed() && ["backlog", "ready"].includes(task.status);
  grip.title = "Drag between Backlog and Ready, or use Edit to change Status";
  grip.addEventListener("dragstart", event => {
    if (!allowed() || locked || !["backlog", "ready"].includes(task.status)) { event.preventDefault(); return; }
    draggedTask = { ...task }; event.dataTransfer.effectAllowed = "move"; event.dataTransfer.setData("text/plain", task.id); card.classList.add("dragging");
    document.querySelectorAll(".board-column, .stage-button").forEach(column => { column.dataset.dropAllowed = String(Boolean(kanbanMovePayload(task, column.dataset.stage, state.runs))); });
  });
  grip.addEventListener("dragend", () => { clearKanbanDrag(); render(); });
  const actions = node("div", null, "task-card-actions"), picker = node("select", null, "task-card-priority");
  picker.setAttribute("aria-label", `Priority for ${task.title || "Untitled task"}`);
  for (const value of [1, 2, 3, 4, 0]) { const option = node("option", priorities[value]); option.value = String(value); picker.append(option); }
  picker.value = String(task.priority ?? 3);
  picker.dataset.command = "true"; picker.dataset.capability = "write"; picker.dataset.unavailable = String(locked);
  picker.disabled = !allowed() || locked;
  picker.title = locked ? "Active and closed tickets keep their recorded specification" : "Change priority";
  picker.addEventListener("change", () => void mutate(`/tasks/${encodeURIComponent(task.id)}`, "PATCH", cardPriorityPayload(task, picker.value), "Priority updated."));
  const edit = commandButton("Edit", () => openEditor(task), { unavailable: locked, title: locked ? picker.title : "Edit ticket" });
  edit.classList.add("task-card-edit"); edit.setAttribute("aria-label", `Edit ${task.title || "Untitled task"}`);
  actions.append(picker, edit); head.append(grip, node("span", shortId(task.id), "task-ref"), actions);
  card.append(head);
  const title = node("div", task.title || "Untitled task", "task-title"); title.title = title.textContent;
  button.append(title);
  const blocker = taskBlocker(task, state.runs);
  if (blocker) { const badge = node("span", blocker.label, `pill task-blocked${blocker.historical ? " historical" : ""}`); badge.title = blocker.reason || "Run is blocked"; button.append(badge); }
  const run = activeRun(task, state.runs);
  const reason = blocker?.reason || (run && activityTitle({ type: "run.updated", status: run.status, stage: run.stage }));
  if (reason) { const summary = node("p", reason, "task-reason"); summary.title = reason; button.append(summary); }
  button.append(node("p",[readable(task.task_type||"feature"),...array(task.labels)].join(" · "),"meta"));
  if (task.workflow === "engineering") button.append(executionSummary(task, state.runs, state.executionActivity));
  const meta = node("div", null, "task-card-meta");
  if (task.project) meta.append(node("span", task.project, "task-project"));
  if(task.criticality)meta.append(node("span",`${task.criticality} bug`,"pill"));
  if (task.owner) { const owner = node("span", null, "task-owner"); owner.append(node("span", task.owner.slice(0, 2).toUpperCase(), "owner-avatar"), node("span", task.owner)); meta.append(owner); }
  if (!task.project && !task.owner) meta.append(node("span", task.workflow === "engineering" ? "Engineering" : "Manual"));
  button.append(meta); button.addEventListener("click", () => void openDetail(task.id)); card.append(button); return card;
}
function setView(view, updateHash = true) {
  state.view = ["work", "attention", "main", "system"].includes(view) ? view : "work";
  const descriptions = { work: "A clear path from intention to outcome.", attention: "The next decision is yours.", main:"From a goal to reviewable tasks.",system:"Your team and current execution activity." };
  $("#view-title").textContent = state.view[0].toUpperCase() + state.view.slice(1);
  $("#view-description").textContent = descriptions[state.view];
  for (const name of ["work", "attention", "main", "system"]) $(`#${name}-view`).hidden = state.view !== name;
  document.querySelectorAll("[data-view]").forEach(button => { const selected = button.dataset.view === state.view; button.classList.toggle("selected", selected); if (selected) button.setAttribute("aria-current", "page"); else button.removeAttribute("aria-current"); });
  renderSystem();renderMain();renderProjects();if(["main","system"].includes(state.view))$("#projects-panel").hidden=true;
  if (updateHash) history.replaceState(null, "", `#${state.view}`);
}
async function openDetail(id) {
  state.selected = id; $("#detail-key").textContent = shortId(id);
  $("#detail-content").replaceChildren(node("p", "Loading task…", "detail-main muted"));
  if (!$("#task-detail").open) $("#task-detail").showModal();
  await loadDetail(id, true);
}
async function loadDetail(id, reportError) {
  const sequence = ++detailRequest;
  try {
    const data = await request(`/tasks/${encodeURIComponent(id)}`, { timeoutMs: 5000 });
    if (sequence !== detailRequest || state.selected !== id || !$("#task-detail").open) return;
    renderDetail(data.task || data, sortedRuns(data.runs || state.runs.filter(run => run.task_id === id)), array(data.events), data.execution_activity);
  } catch (error) { if (reportError && sequence === detailRequest) $("#detail-content").replaceChildren(node("p", errorMessage(error), "detail-main form-error")); }
}
function commandButton(label, handler, { capability = "write", unavailable = false, primary = false, danger = false, title } = {}) {
  unavailable = Boolean(unavailable);
  const button = node("button", label, primary ? "primary-button" : `secondary-button${danger ? " danger-button" : ""}`);
  button.dataset.command = "true"; button.dataset.capability = capability; button.dataset.unavailable = String(unavailable);
  if (title) button.title = title;
  button.disabled = !allowed(capability) || unavailable;
  button.addEventListener("click", handler); return button;
}
function section(title) { const block = node("section", null, "detail-section"); block.append(node("h3", title)); return block; }
function property(list, name, value, href) {
  list.append(node("dt", name)); const item = node("dd");
  if (href && safeWebUrl(href)) { const link = node("a", value); link.href = safeWebUrl(href); link.target = "_blank"; link.rel = "noreferrer"; item.append(link); }
  else item.textContent = value || "—";
  list.append(item);
}
function renderDetail(task, runs, events, executionActivity) {
  const root = node("div", null, "detail-main"), tags = node("div", null, "detail-tags"), status = taskStatus(task, runs);
  tags.append(node("span", labels[status], `pill ${status === "active" ? "working" : status === "review" ? "paused" : ""}`), node("span", priorities[task.priority] || "Normal", "pill"), node("span", repositoryFreeResearch(task) ? "Research" : task.workflow === "engineering" ? "Engineering" : "Manual", "pill"));
  const title = node("h2", task.title, "detail-title"); title.id = "detail-title";
  if(task.archived_at)tags.append(node("span","Archived","pill"));
  const blocker = taskBlocker(task, runs);
  if (blocker) tags.append(node("span", blocker.label, `pill task-blocked${blocker.historical ? " historical" : ""}`));
  root.append(tags, title);
  const actions = node("div", null, "detail-actions"), running = activeRun(task, runs);
  actions.append(commandButton("Edit task", () => openEditor(task), { unavailable: Boolean(running) || task.archived_at || ["done","cancelled"].includes(task.status), title: running ? "Scope is locked while a run is active" : ["done","cancelled"].includes(task.status) ? "Closed tasks retain their completed specification" : undefined }));
  if (!running && task.status === "backlog") actions.append(commandButton("Mark ready", () => void mutate(`/tasks/${encodeURIComponent(task.id)}`, "PATCH", readyTaskPayload(task), task.automation_policy === "ready_to_done" ? "Task queued for automatic processing." : "Task is ready.")));
  if (task.workflow === "engineering" && !running && !["done","cancelled"].includes(task.status) && !(task.automation_policy === "ready_to_done" && task.automation_revision === task.revision)) {
    const retry = runs.length > 0;
    actions.append(commandButton(retry ? "Retry run" : "Run task", () => void mutate(`/tasks/${encodeURIComponent(task.id)}/run`, "POST", { expected_revision: task.revision }, "Run requested. Check its recorded status below."), { primary: true, capability: "run", unavailable: !["ready", "review"].includes(task.status) || Boolean(task.blocked_reason)||Boolean(task.dependency_blockers?.length), title: !["ready", "review"].includes(task.status) ? "Mark this task ready before requesting a run" : task.dependency_blockers?.length ? dependencyReason(task) : state.capabilities.run !== true ? "Execution is paused, busy, or needs attention" : undefined }));
  }
  if (!running && task.workflow === "manual" && ["backlog", "ready", "review"].includes(task.status)) actions.append(commandButton("Accept result", () => void mutate(`/tasks/${encodeURIComponent(task.id)}/accept`, "POST", { expected_revision: task.revision }, "Result accepted."), { capability: "accept", primary: true, unavailable: Boolean(task.blocked_reason) }));
  if(!task.archived_at && ["done","cancelled"].includes(task.status))actions.append(commandButton("Archive",()=>void mutate(`/tasks/${encodeURIComponent(task.id)}/archive`,"POST",{expected_revision:task.revision},"Task archived.")));
  if(task.archived_at)actions.append(commandButton("Restore",()=>void mutate(`/tasks/${encodeURIComponent(task.id)}/restore`,"POST",{expected_revision:task.revision},"Task restored.")));
  if(!running&&["done","cancelled"].includes(task.status))actions.append(commandButton("Reopen to Backlog",()=>void mutate(`/tasks/${encodeURIComponent(task.id)}/reopen`,"POST",{expected_revision:task.revision},"Task reopened. No run was started.")));
  root.append(actions);
  if (blocker) root.append(node("p", `${blocker.label}: ${blocker.reason || "No reason recorded."}`, "notice"));
  if (task.workflow === "engineering") root.append(executionSummary(task, runs, executionActivity, true));
  const queue=state.projects.find(p=>p.repository?.toLowerCase()===task.repository?.toLowerCase());
  if(queue?.runtime_status==="preparing")root.append(node("p",state.codingAgents ? "Preparing this repository for the assigned coding agent. No worker has started." : "Preparing this repository’s checks and offline dependencies. No worker has started.","notice"));
  if(queue?.failure)root.append(node("p",`Repository queue paused: ${errorMessage({code:queue.failure})}`,"notice"));
  if (task.workflow === "engineering" && !running && !["done","cancelled"].includes(task.status)) {
    const notice = node("p", null, "detail-empty"); notice.dataset.executionNotice = "true"; root.append(notice);
  }
  root.append(linkedText("p", task.objective || "No objective recorded.", "detail-description"));
  const criteria = section("Acceptance criteria"), list = node("ul", null, "criteria-list");
  array(task.acceptance_criteria).forEach(item => list.append(linkedText("li", item))); criteria.append(list.childElementCount ? list : node("p", "No acceptance criteria recorded.", "detail-empty")); root.append(criteria);
  const details = section("Details"), properties = node("dl", null, "detail-properties");
  property(properties, "Criticality", task.criticality);
  if(task.qa_source?.task_id){const source=node("button","View source task","text-button");source.type="button";source.addEventListener("click",()=>void openDetail(task.qa_source.task_id));root.append(source,node("p",`Reported by QA on ${task.qa_source.reviewed_sha || ""}`,"meta"));}
  if (task.coding_agent_id) property(properties, "Coding agent", array(state.codingAgents?.agents).find(a => a.id === task.coding_agent_id)?.label || task.coding_agent_id);
  property(properties, "Project", task.project); property(properties, "Owner", task.owner); property(properties, "Revision", String(task.revision ?? "—"));
  if (task.repository) property(properties, "Repository", task.repository, `https://github.com/${task.repository}`);
  if (safeGithubTaskUrl(task.github_issue_url, "issues")) property(properties, "GitHub issue", task.github_issue_url, task.github_issue_url);
  if (safeGithubTaskUrl(task.github_pull_request_url, "pull")) property(properties, "GitHub PR", task.github_pull_request_url, task.github_pull_request_url);
  property(properties,"Type",readable(task.task_type||"feature")); if(task.labels?.length)property(properties,"Labels",task.labels.join(", "));
  if(task.dependencies?.length){const dependencySection=section("Dependencies");for(const id of task.dependencies){const dep=state.tasks.find(t=>t.id===id);dependencySection.append(commandButton(`${dep?.title||id} · ${readable(dep?.status||"missing")}`,()=>void openDetail(id)));}root.append(dependencySection);}
  if (task.workflow === "engineering") { property(properties, "QA", task.qa_policy === "skip" ? "Explicitly skipped" : "Required"); property(properties, "Delivery", repositoryFreeResearch(task)?"Research report · no PR":readable(task.merge_policy || "manual")); property(properties, "Automation", automationLabel(task, latestRun(task, runs))); }
  if(task.completed_at)property(properties,"Completed",displayHistoricalTime(task.completed_at));
  if(task.cancelled_at)property(properties,"Canceled",displayHistoricalTime(task.cancelled_at));
  if(task.source_id)property(properties,"Source",task.source_id,task.source_url);
  if(task.source_history?.status_name)property(properties,"Original status",task.source_history.status_name);
  if(task.source_history?.created_at)property(properties,"Originally created",displayHistoricalTime(task.source_history.created_at));
  if(task.source_history?.updated_at)property(properties,"Originally updated",displayHistoricalTime(task.source_history.updated_at));
  if(task.source_history?.completed_at)property(properties,"Original completion",displayHistoricalTime(task.source_history.completed_at));
  if(task.source_history?.cancelled_at)property(properties,"Original cancellation",displayHistoricalTime(task.source_history.cancelled_at));
  if(task.source_history?.labels?.length)property(properties,"Original labels",task.source_history.labels.join(", "));
  details.append(properties); root.append(details);
  if(task.source_description){const original=section("Original specification (reference)");original.append(node("p","Review this source material into the objective, context, and acceptance criteria before execution.","meta"),linkedText("pre",task.source_description,"source-description"));root.append(original);}
  if (array(task.context).length) { const context = section("Context"); for (const item of task.context) context.append(linkedText("p", item, "detail-description")); root.append(context); }
  if (task.workflow === "engineering") {
  const history = section("Runs");
  if (!runs.length) history.append(node("p", "No runs requested. Tasks start with an explicit Run or a separately activated, explicitly enrolled Ready revision.", "detail-empty"));
  runs.forEach((run, index) => {
    const row = node("article", null, "run-row"), head = node("div", null, "run-row-head");
    head.append(node("span", `Run ${runs.length - index}`, "run-name"), node("span", readable(run.status), `pill ${["failed", "blocked", "unknown"].includes(run.status) ? "failed" : ["running", "admitted"].includes(run.status) ? "working" : ""}`));
    row.append(head, node("p", `${shortId(run.id)} · task revision ${run.task_revision ?? "—"} · ${displayTime(run.created_at)}`, "meta"));
    if(run.kind==="research" && run.report){
      const report=node("section",null,"research-report");report.append(node("h4",run.status==="succeeded"?"Research report":"Research report · awaiting verification"),node("pre",run.report.markdown,"research-text"),node("h4","Sources"));
      for(const source of array(run.report.sources)){const link=node("a",source.title);link.href=source.url;link.rel="noreferrer noopener";link.target="_blank";report.append(link,node("br"));}
      if(run.status==="succeeded")for(const name of ["report.md","sources.json"]){const link=node("a",`Download ${name}`,"artifact-link");link.href=`${api}/runs/${encodeURIComponent(run.id)}/${name}`;link.download="";report.append(link);}
      row.append(report);
    }
    if (run.head_sha) row.append(node("p", `Commit ${run.head_sha}`, "meta"));
    if(run.repair_count)row.append(node("p",`QA repairs ${run.repair_count} of ${run.max_qa_repairs ?? 0}`,"meta"));
    for(const cycle of array(run.review_history)){const review=node("div",null,"meta");review.append(node("p",`QA cycle ${Number(cycle.cycle??array(run.review_history).indexOf(cycle))+1}: ${cycle.passed ? "Passed" : "Changes requested"} · ${cycle.reviewed_sha || cycle.report_digest || ""}`));for(const finding of array(cycle.findings))review.append(node("p",typeof finding==="string"?finding:`${finding.path}:${finding.line} — ${finding.summary}`));for(const [i,bug] of array(cycle.out_of_scope_bugs).entries()){review.append(node("p",`Unrelated bug: ${qaBugLabel(bug)}`));review.append(node("p",`Reproduce: ${bug.reproduction} · Expected: ${bug.expected} · Actual: ${bug.actual}`));review.append(node("p",`Base ${bug.base_sha}: ${bug.base_reproduction} · Unrelated because: ${bug.unrelated_reason}`));const id=cycle.bug_task_ids?.[i];if(id){const link=node("button","View Backlog bug","text-button");link.type="button";link.addEventListener("click",()=>void openDetail(id));review.append(link);}}row.append(review);}

    if (run.deadline_ms) row.append(node("p", `Deadline ${displayTime(run.deadline_ms)}`, "meta"));
    if (run.stage || run.reason || run.blocked_reason) row.append(node("p", executionReason(run.reason || run.blocked_reason) || activityTitle({ type: "run.updated", status: run.status, stage: run.stage }), "meta"));
    for (const [title, href] of [["Pull request", run.github_url || run.pull_request_url], ["Session", run.session_url], ["Evidence", run.evidence_url]]) if (safeArtifactUrl(href, location.origin)) { const link = node("a", title, "artifact-link meta"); link.href = safeArtifactUrl(href, location.origin); link.rel = "noreferrer"; link.target = "_blank"; row.append(link); }
    array(run.evidence?.artifact_urls).forEach((href, artifactIndex) => { const accepted = safeArtifactUrl(href, location.origin); if (!accepted) return; const link = node("a", `Artifact ${artifactIndex + 1}`, "artifact-link meta"); link.href = accepted; link.rel = "noreferrer"; link.target = "_blank"; row.append(link); });
    if (run.evidence?.qa_status || run.evidence?.review_status) row.append(node("p", `Review: ${readable(run.evidence.review_status)} · QA: ${readable(run.evidence.qa_status)}`, "meta"));
    if (["unknown", "awaiting_merge"].includes(run.status)) row.append(commandButton(run.status === "awaiting_merge" ? "Check merge" : "Reconcile outcome", () => void mutate(`/runs/${encodeURIComponent(run.id)}/reconcile`, "POST", {}, "Outcome check requested. Recorded actions will not be relaunched."), { capability: "write" }));
    if (run.status === "queued") row.append(commandButton("Cancel queued run", () => void mutate(`/runs/${encodeURIComponent(run.id)}/cancel`, "POST", {}, "Cancellation recorded. Check the run status."), { capability: "cancel", danger: true }));
    if (["admitted", "running", "blocked", "paused", "awaiting_merge", "unknown"].includes(run.status)) row.append(commandButton("Request stop", () => void mutate(`/runs/${encodeURIComponent(run.id)}/cancel`, "POST", {}, "Stop requested. Execution remains active until the host confirms termination."), { capability: "stop", danger: true, title: state.capabilities.stop !== true ? "The execution host does not support a confirmed stop" : undefined }));
    if (run.status === "awaiting_merge") row.append(node("p", "Merge this PR manually, then check its recorded outcome here.", "task-reason"));
    if (run.status === "unknown") row.append(node("p", "The last action may have taken effect. Reconciliation checks its outcome without relaunching it.", "task-reason"));
    if (run.status === "cancel_requested") row.append(node("p", "Stop requested. Waiting for confirmed termination and cleanup.", "task-reason"));
    history.append(row);
  }); root.append(history);
  }
  const activity = section("Activity"), timeline = node("ol", null, "event-timeline");
  coalesceActivity(events.slice().reverse(), displayTime).slice(0, 50).forEach(event => { const item = node("li"); item.append(node("span", event.summary || activityTitle(event)), node("time", displayTime(event.at || event.occurred_at))); if (event.reason) item.append(node("p", executionReason(event.reason), "meta")); timeline.append(item); });
  activity.append(events.length ? timeline : node("p", "No recorded activity yet.", "detail-empty")); root.append(activity);
  $("#detail-content").replaceChildren(root); $("#detail-key").textContent = `${shortId(task.id)} · revision ${task.revision ?? "—"}`;
  updateControls();
}
function openEditor(task = null, projectId = null) {
  if (!allowed() || (task && (activeRun(task, state.runs) || ["done","cancelled"].includes(task.status) || task.archived_at))) return;
  state.editing = task; const form = $("#task-form"); form.reset();
  form.elements.workflow.value = task?.workflow || "engineering";
  editorQaExplicit=Boolean(task);
  form.elements.task_type.value=task?.task_type||"feature";
  form.elements.qa_policy.value=task?.qa_policy||"required";
  form.elements.labels.value=array(task?.labels).join(", ");
  const dependencies=form.elements.dependencies;dependencies.replaceChildren();for(const dep of state.tasks.filter(t=>t.id!==task?.id)){const option=node("option",`${dep.title} · ${readable(dep.status)}`);option.value=dep.id;option.selected=array(task?.dependencies).includes(dep.id);dependencies.append(option);}
  if(!task)form.elements.owner.value="Coder";
  const coding = codingAgentSelection(state.codingAgents, task), picker = form.elements.coding_agent_id;
  picker.replaceChildren();
  if (!coding.selected) { const option = node("option", coding.unavailable ? "Choose an available coding agent" : "Choose a coding agent"); option.value = ""; picker.append(option); }
  for (const agent of coding.agents) { const option = node("option", agent.label); option.value = agent.id; picker.append(option); }
  picker.value = coding.selected;
  form.elements.automation_policy.value = initialAutomationPolicy(state.capabilities, Boolean(task), task);
  form.elements.merge_policy.value = task?.merge_policy || "auto";
  form.elements.workflow.querySelector('option[value="manual"]').disabled = task?.workflow === "engineering";
  if(!task && projectId){const p=state.projects.find(p=>p.id===projectId);if(p){form.elements.repository.value=p.repository;form.elements.project.value=p.name;}}
  $("#editor-title").textContent = task ? "Edit task" : "New task";
  $("#save-task").textContent = task ? "Save changes" : "Create task";
  $("#editor-error").textContent = "";
  $("#task-status option[value='review']")?.remove();
  if (task) {
    for (const name of ["title", "objective", "project", "owner", "priority", "criticality", "status", "workflow", "repository", "blocked_reason", "github_issue_url", "github_pull_request_url", "merge_policy"]) form.elements[name].value = task[name] ?? "";
    if (task.status === "review") form.elements.status.value = "ready";
    form.elements.acceptance_criteria.value = array(task.acceptance_criteria).join("\n"); form.elements.context.value = array(task.context).join("\n");
  }
  updateWorkflowHint(); if (!$("#task-editor").open) $("#task-editor").showModal(); $("#task-title").focus();
}
function updateWorkflowHint() {
  const engineering = $("#task-workflow").value === "engineering";
  const report = $("#task-type").value === "research" && !$("#task-repository").value.trim();
  $("#task-workflow").querySelector('option[value="engineering"]').textContent = report ? "Research · agent → report" : "Engineering · Coder → required checks → delivery";
  $("#editor-queue-hint").textContent = !engineering ? "Manual tasks record work without starting an agent." : report ? "Set Research to Ready to start web research and receive a report with sources." : "Set an engineering task to Ready to queue Coder, required checks and delivery for its repository.";
  $("#repository-field").hidden = !engineering; $("#task-repository").required = engineering && $("#task-type").value!=="research";
  $("#automation-field").hidden = !engineering || state.capabilities.ready_to_done !== true;
  const hasCodingAgents = codingAgentSelection(state.codingAgents).agents.length > 0;
  $("#coding-agent-field").hidden = !engineering || !hasCodingAgents;
  $("#task-coding-agent").disabled = !engineering || !hasCodingAgents;
  $("#task-coding-agent").required = engineering && hasCodingAgents;
  $("#merge-policy-field").hidden = !engineering || ($("#task-type").value==="research" && !$("#task-repository").value.trim()); $("#qa-policy-field").hidden=!engineering;
  $("#workflow-hint").textContent = engineering && $("#task-type").value==="research" && !$("#task-repository").value.trim() ? "Repository-free research: Ready starts web research and produces a report with sources and downloads. No GitHub changes or PR. Required QA reviews the exact report; Skip QA is the default." : engineering ? hasCodingAgents ? "Enter owner/repository and choose a coding agent. When Ready, the agent finds or prepares the project environment, then implements the task. QA and delivery follow your choices." : "Enter owner/repository. Ready tasks discover its checks and queue Coder, required checks and delivery. A failure pauses only that repository." : "Manual tasks record work without starting an agent.";
}
async function mutate(path, method, body, message) {
  if (!allowed(method === "POST" && path.endsWith("/run") ? "run" : "write")) return;
  state.busy = true; updateControls();
  try { const result = await request(path, { method, body }); notify(message); await refresh(); return result; }
  catch (error) { notify(errorMessage(error)); await refresh(); }
  finally { state.busy = false; updateControls(); if (state.selected && $("#task-detail").open) await loadDetail(state.selected, false); }
}

document.querySelectorAll("[data-view]").forEach(button => button.addEventListener("click", () => setView(button.dataset.view)));
$(".brand").addEventListener("click", () => setView("work"));
for (const selector of ["#task-search", "#project-filter", "#owner-filter", "#type-filter", "#label-filter", "#archive-filter"]) $(selector).addEventListener(selector === "#task-search" ? "input" : "change", render);
$("#reset-filters").addEventListener("click",resetBoardFilters);
$("#board-filter-menu").addEventListener("keydown",event=>{if(event.key==="Escape"){$("#board-filter-menu").open=false;$("#board-filter-toggle").focus();}});
document.addEventListener("click",event=>{const menu=$("#board-filter-menu");if(menu.open&&!menu.contains(event.target))menu.open=false;});
$("#board-stages").addEventListener("click", event => {
  const button = event.target.closest("[data-stage]");
  if (!button || !columns.includes(button.dataset.stage)) return;
  state.stage = button.dataset.stage;
  try { localStorage.setItem("workboard-stage", state.stage); } catch { /* Selection remains available for this page. */ }
  render();
});
$("#new-task").addEventListener("click", () => openEditor()); $("#empty-create").addEventListener("click", () => openEditor());
$("#close-detail").addEventListener("click", () => $("#task-detail").close());
$("#task-detail").addEventListener("close", () => { state.selected = null; detailRequest++; });
$("#close-editor").addEventListener("click", () => $("#task-editor").close());
$("#workflow-hint").before($("#automation-field"));
$("#task-workflow").addEventListener("change", updateWorkflowHint);
$("#task-form").addEventListener("submit", async event => {
  event.preventDefault(); if (!allowed()) return;
  const task = state.editing, fields=Object.fromEntries(new FormData(event.currentTarget));fields.dependencies=new FormData(event.currentTarget).getAll("dependencies");const body = taskPayload(fields);
  if (task) body.expected_revision = task.revision; else { body.merge_policy = body.workflow === "engineering" ? body.merge_policy || "auto" : "manual"; }
  state.busy = true; updateControls(); $("#editor-error").textContent = "";
  try {
    const result = await request(task ? `/tasks/${encodeURIComponent(task.id)}` : "/tasks", { method: task ? "PATCH" : "POST", body });
    $("#task-editor").close(); notify(body.automation_policy === "ready_to_done" ? body.status === "ready" ? "Task queued for automatic processing; paused launches stay paused." : "Task saved. Mark it Ready to join its queue." : task ? "Task updated." : "Task saved for an explicit Run."); await refresh();
    if (!task && result.task?.id) await openDetail(result.task.id);
  } catch (error) { $("#editor-error").textContent = errorMessage(error); }
  finally { state.busy = false; updateControls(); }
});
$("#execution-pause").addEventListener("click", () => {
  if (state.execution?.approved) void mutate("/execution/pause", "POST", { paused: !state.execution.paused }, state.execution.paused ? "Launches resumed." : "New launches paused.");
});
$("#login-form").addEventListener("submit", async event => {
  event.preventDefault(); const button = event.currentTarget.querySelector("button"); button.disabled = true; $("#login-error").textContent = "";
  try { applySession(await request("/session", { method: "POST", body: { key: $("#operator-key").value } })); $("#operator-key").value = ""; if (state.authenticated) { await refresh(); connectEvents(); } else $("#login-error").textContent = "Access was not accepted."; }
  catch (error) { $("#operator-key").value = ""; $("#login-error").textContent = error.network ? "Host unavailable. Check your connection." : "Access was not accepted. Check the operator key."; }
  finally { button.disabled = false; }
});
$("#sign-out").addEventListener("click", async () => { try { await request("/session", { method: "DELETE", body: {} }); expireSession(); state.plans=[];planEdits.clear();state.projects = []; state.tasks = []; state.runs = []; state.events = []; state.executionActivity = {state:"unavailable",tasks:{}}; state.loaded = false; render(); $("#board-freshness").textContent = "Signed out"; } catch (error) { notify(errorMessage(error)); } });
window.addEventListener("offline", () => setConnection(false));
window.addEventListener("online", () => { if (state.authenticated) void refresh(); else void bootstrap(); });
window.addEventListener("hashchange", () => setView(location.hash.slice(1), false));
document.addEventListener("visibilitychange", () => { if (!document.hidden && state.authenticated) void refresh(); });
setView(location.hash.slice(1), false); void bootstrap();
setInterval(() => { if (!document.hidden && state.authenticated) void refresh(); }, 60000);

function renderProjects(){
 const panel=$("#projects-panel");panel.hidden=!state.authenticated||!state.projects.length;
 const list=$("#project-list"),expanded=new Set([...list.querySelectorAll("details[open]")].map(row=>row.id)),counts=new Map();
 list.replaceChildren();$("#projects-summary").textContent="";if(!state.authenticated)return;
 for(const project of state.projects){
  const queued=state.tasks.filter(t=>(t.project_id===project.id||t.repository?.toLowerCase()===project.repository?.toLowerCase())&&t.status==="ready"&&t.automation_policy==="ready_to_done"&&t.automation_revision===t.revision).length;
  const status=repositoryQueueStatus(project,queued,state.execution?.paused);counts.set(status,(counts.get(status)||0)+1);
  const card=node("details",null,"repository-queue");card.id=`queue-${project.id}`;card.open=expanded.has(card.id);
  const summary=node("summary",null,"queue-summary"),head=node("span",null,"queue-summary-content");head.append(node("span",project.repository,"queue-repository"),node("span",status,"queue-status meta"),node("span",`${queued} queued`,"meta"));summary.append(head);card.append(summary);
  const content=node("div",null,"queue-details");content.append(node("p",project.validation_summary||"Contained validation","meta"),node("p",`Fresh remote clone · ${project.default_branch||"default branch"}`,"meta"));
  if(state.execution?.paused)content.append(node("p","All launches are paused. Resume launches above when ready.","meta"));
  if(project.failure)content.append(node("p",`This repository is paused: ${errorMessage({code:project.failure})}. Other repositories continue.`,"notice"));
  const controls=node("div",null,"detail-actions");controls.append(commandButton(project.paused?"Resume queue":"Pause queue",()=>void mutate(`/projects/${encodeURIComponent(project.id)}`,"PATCH",{expected_revision:project.revision,paused:!project.paused},project.paused?"Repository queue resumed.":"Repository launches paused.")),commandButton("New task",()=>openEditor(null,project.id),{primary:true}));controls.append(commandButton("Prepare checks again",()=>void mutate(`/projects/${encodeURIComponent(project.id)}`,"PATCH",{expected_revision:project.revision,reconnect:true},"Repository preparation queued. Ready tasks will continue after checks are prepared."),{unavailable:Boolean(project.lane_run_id)||project.runtime_status==="preparing"}));content.append(controls);card.append(content);list.append(card);
 }
 $("#projects-summary").textContent=["Needs attention","Preparing checks","Working","Queued","Paused","Idle"].filter(status=>counts.has(status)).map(status=>`${counts.get(status)} ${status.toLowerCase()}`).join(" · ");
}

$("#task-merge-policy").addEventListener("change",updateWorkflowHint);
$("#task-qa-policy").addEventListener("change",()=>{editorQaExplicit=true;});
$("#task-type").addEventListener("change",()=>{if(!editorQaExplicit)$("#task-qa-policy").value=defaultQaPolicy($("#task-type").value);updateWorkflowHint();});
$("#task-repository").addEventListener("input",updateWorkflowHint);

function renderMain(){
 const active=state.plans.some(p=>["queued","generating","unknown","cleanup_pending"].includes(p.status));
 $("#main-generate").disabled=!state.authenticated||!state.capabilities.planning||state.execution?.paused!==false||active||state.busy;
 $("#main-status").textContent=!state.authenticated?"Sign in to plan.":!state.capabilities.planning?"AI planning is not configured on this host.":active?"Finish or discard the current plan before generating another.":state.execution?.paused!==false?"Resume launches above before generating an AI plan.":"Generation uses AI once. Accepted tasks stay in Backlog until you mark them Ready.";
 const list=$("#main-plans");list.replaceChildren();if(!state.authenticated)return;
 for(const plan of state.plans){
  const card=node("article",null,"panel project-card");card.append(node("h3",plan.goal),node("p",`${plan.project} · ${plan.repository} · ${readable(plan.status)}`,"meta"),node("p","Goal-based plan · repository source not inspected","meta"));
  if(plan.context)card.append(node("p",`Context: ${plan.context}`,"meta"));
  if(plan.summary)card.append(node("p",plan.summary));
  if(plan.status==="proposed"){
   for(const p of array(plan.proposals)){
    if(!planEdits.has(p.proposal_id))planEdits.set(p.proposal_id,{selected:true,title:p.title,objective:p.objective,acceptance_criteria:array(p.acceptance_criteria).join("\n"),priority:p.priority,task_type:p.task_type||"feature",labels:array(p.labels).join(", "),depends_on:array(p.depends_on),qa_policy:defaultQaPolicy(p.task_type),qa_explicit:false});
    const edit=planEdits.get(p.proposal_id),section=node("fieldset",null,"form-fields"),check=node("input");check.type="checkbox";check.checked=edit.selected;check.addEventListener("change",()=>{edit.selected=check.checked;});const choose=node("label","Add this task to Backlog");choose.prepend(check);section.append(choose);
    for(const [name,label,multiline,max] of [["title","Title",false,240],["objective","Objective",true,4000],["acceptance_criteria","Acceptance criteria (one per line)",true,8000]]){const field=node(multiline?"textarea":"input");field.value=edit[name];field.maxLength=max;field.setAttribute("aria-label",label);field.addEventListener("input",()=>edit[name]=field.value);const wrap=node("label",label);wrap.append(field);section.append(wrap);}
    const type=node("select");type.setAttribute("aria-label","Task type");for(const value of ["feature","bug","improvement","chore","research"]){const o=node("option",readable(value));o.value=value;type.append(o);}type.value=edit.task_type;const qa=node("select");qa.setAttribute("aria-label","QA");for(const [value,text]of [["required","Required QA"],["skip","Skip independent QA"]]){const o=node("option",text);o.value=value;qa.append(o);}qa.value=edit.qa_policy;qa.addEventListener("change",()=>{edit.qa_policy=qa.value;edit.qa_explicit=true;});type.addEventListener("change",()=>{edit.task_type=type.value;if(!edit.qa_explicit){edit.qa_policy=defaultQaPolicy(type.value);qa.value=edit.qa_policy;}});const typeLabel=node("label","Type");typeLabel.append(type);const qaLabel=node("label","QA");qaLabel.append(qa);section.append(typeLabel,qaLabel);
    const tags=node("input");tags.setAttribute("aria-label","Labels");tags.value=edit.labels;tags.addEventListener("input",()=>edit.labels=tags.value);const tagsLabel=node("label","Labels (comma separated)");tagsLabel.append(tags);section.append(tagsLabel);
    const deps=node("select");deps.multiple=true;deps.setAttribute("aria-label","Prerequisite proposals");array(plan.proposals).forEach((other,index)=>{if(other.proposal_id===p.proposal_id)return;const o=node("option",`${index+1}. ${other.title}`);o.value=String(index);o.selected=edit.depends_on.includes(index);deps.append(o);});deps.addEventListener("change",()=>edit.depends_on=[...deps.selectedOptions].map(o=>Number(o.value)));const depsLabel=node("label","Prerequisite proposals (select them for acceptance too)");depsLabel.append(deps);section.append(depsLabel);
    const priority=node("select");priority.setAttribute("aria-label","Priority");for(const [value,label]of [[1,"Urgent"],[2,"High"],[3,"Normal"],[4,"Low"]]){const option=node("option",label);option.value=String(value);priority.append(option);}priority.value=String(edit.priority);priority.addEventListener("change",()=>edit.priority=Number(priority.value));const label=node("label","Priority");label.append(priority);section.append(label);card.append(section);
   }
   const accept=commandButton("Add selected to Backlog",()=>{const selected=planSelection(array(plan.proposals),planEdits);if(!selected.length){notify("Select at least one proposed task.");return;}void mutate(`/plans/${encodeURIComponent(plan.id)}/accept`,"POST",{expected_revision:plan.revision,selected},"Selected tasks added to Backlog. No run was started.");},{primary:true});card.append(accept);
  }
  if(plan.status==="accepted")for(const id of array(plan.accepted_task_ids))card.append(commandButton("View accepted task",()=>void openDetail(id)));
  if(["unknown","queued","generating","proposed"].includes(plan.status))card.append(commandButton(plan.status==="unknown"?"Check cleanup and discard":"Discard plan",()=>void mutate(`/plans/${encodeURIComponent(plan.id)}/discard`,"POST",{},"Discard requested. Generation will not be replayed."),{danger:true}));
  if(plan.status==="unknown")card.append(node("p","Generation was interrupted or its result could not be verified. Check cleanup and discard before generating another plan.","notice"));
  list.append(card);
 }
}
$("#main-goal-form").addEventListener("submit",async event=>{event.preventDefault();if(!state.authenticated)return;const body=Object.fromEntries(new FormData(event.currentTarget));$("#main-error").textContent="";state.busy=true;renderMain();try{await request("/plans",{method:"POST",body});await refresh();notify("Main is generating proposals. No tasks or runs have been created.");}catch(error){$("#main-error").textContent=errorMessage(error);}finally{state.busy=false;renderMain();}});


function agentAssignments(agent){
 const assignments=[];
 for(const run of state.runs){const role=String(run.stage||"").startsWith("qa")?"qa":String(run.stage||"").startsWith("coder")?"coder":null;if(role&&liveRunStates.has(run.status)&&run.agents?.[role]?.id===agent.id)assignments.push(`${readable(run.status)}: ${state.tasks.find(t=>t.id===run.task_id)?.title||"Retained task"}`);}
 for(const plan of state.plans)if(["queued","generating","unknown","cleanup_pending"].includes(plan.status)&&plan.agent?.id===agent.id)assignments.push(`${readable(plan.status)}: ${plan.goal}`);
 return assignments;
}
function renderSystem(){
 const system=state.system,list=$("#system-agents"),activity=$("#system-activity");list.replaceChildren();activity.replaceChildren();
 $("#edit-agents").disabled=!allowed()||!system?.editable;
 $("#system-status").textContent=!state.authenticated?"Sign in to view your team.":!system?"Agent settings are unavailable on this host.":`${state.connected?"Connected":"Connection interrupted"} · ${!state.execution?.approved?"Execution unavailable":state.execution.paused?"Launches paused":"Launches enabled"}`;
 if(!state.authenticated||!system)return;
 const roster=[...array(system.agents)];
 for(const pinned of [...state.runs.flatMap(run=>Object.values(run.agents||{})),...state.plans.map(plan=>plan.agent)])if(pinned?.id&&agentAssignments(pinned).length&&!roster.some(agent=>agent.id===pinned.id))roster.push({...pinned,enabled:false,retained:true});
 for(const agent of roster){
  const card=node("article",null,"panel"),defaultRoles=Object.entries(system.defaults||{}).filter(([,id])=>id===agent.id).map(([role])=>readable(role));
  card.append(node("h3",agent.label),node("p",agentAssignments(agent).join(" · ")||"No active assignment","meta"),node("p",`${readable(agent.role)} · ${agent.retained?"Retained for current work":agent.enabled?"Enabled":"Disabled"}${defaultRoles.length?" · Default: "+defaultRoles.join(", "):""}`,"meta"),node("p",agent.model||"Host default model","meta"),node("p",agent.instructions||"No additional instructions.","agent-instructions"));list.append(card);
 }
 for(const run of state.runs.slice(0,10)){
  const task=state.tasks.find(t=>t.id===run.task_id),row=node("article",null,"system-event");
  row.append(node("strong",task?.title||"Retained task"),node("p",`${readable(run.stage||run.status)} · ${readable(run.status)} · ${displayHistoricalTime(run.updated_at||run.created_at)}`,"meta"));
  if(task)row.append(commandButton("Open task",()=>void openDetail(task.id)));activity.append(row);
 }
 if(!state.runs.length)activity.append(node("p","No execution recorded yet.","meta"));
}
function renderAgentDraft(){
 const fields=$("#agent-fields"),defaults=$("#agent-defaults");fields.replaceChildren();defaults.replaceChildren();if(!agentDraft)return;
 for(const role of ["main","coder","qa"]){const label=node("label",`Default ${role}`),select=node("select");select.setAttribute("aria-label",`Default ${role}`);for(const agent of agentDraft.agents.filter(a=>a.role===role)){const option=node("option",agent.label+(agent.enabled?"":" (disabled)"));option.value=agent.id;select.append(option);}select.value=agentDraft.defaults[role]||"";select.addEventListener("change",()=>agentDraft.defaults[role]=select.value);label.append(select);defaults.append(label);}
 for(const agent of agentDraft.agents){
  const group=node("fieldset",null,"agent-editor-fields");group.append(node("legend",agent.id));
  for(const [key,title,multiline]of [["label","Name",false],["instructions","Instructions",true],["model","Model (blank uses host default)",false]]){const label=node("label",title),input=node(multiline?"textarea":"input");input.value=agent[key]||"";input.maxLength=key==="instructions"?2000:80;input.setAttribute("aria-label",`${title} for ${agent.id}`);input.addEventListener("input",()=>{agent[key]=input.value;});label.append(input);group.append(label);}
  const roleLabel=node("label","Role"),role=node("select");role.setAttribute("aria-label",`Role for ${agent.id}`);for(const value of ["main","coder","qa"]){const option=node("option",readable(value));option.value=value;role.append(option);}role.value=agent.role;role.addEventListener("change",()=>{agent.role=role.value;renderAgentDraft();});roleLabel.append(role);group.append(roleLabel);
  const enabledLabel=node("label","Enabled"),enabled=node("input");enabled.type="checkbox";enabled.checked=agent.enabled;enabled.addEventListener("change",()=>{agent.enabled=enabled.checked;renderAgentDraft();});enabledLabel.prepend(enabled);group.append(enabledLabel);
  const remove=node("button","Remove agent","secondary-button");remove.type="button";remove.addEventListener("click",()=>{agentDraft.agents=agentDraft.agents.filter(a=>a!==agent);renderAgentDraft();});group.append(remove);fields.append(group);
 }
}
$("#edit-agents").addEventListener("click",()=>{if(!allowed()||!state.system?.editable)return;agentDraft=structuredClone(state.system);$("#agents-error").textContent="";renderAgentDraft();$("#agents-editor").showModal();});
$("#close-agents").addEventListener("click",()=>{$("#agents-editor").close();agentDraft=null;});
$("#add-agent").addEventListener("click",()=>{agentDraft.agents.push({id:`agent-${crypto.randomUUID().slice(0,8)}`,label:"New agent",role:"coder",instructions:"",model:"",enabled:true});renderAgentDraft();});
$("#agents-form").addEventListener("submit",async event=>{event.preventDefault();if(!agentDraft||!allowed())return;$("#save-agents").disabled=true;try{await request("/system/agents",{method:"PUT",body:{expected_revision:agentDraft.revision,agents:agentDraft.agents,defaults:agentDraft.defaults}});$("#agents-editor").close();agentDraft=null;await refresh();notify("Agent settings saved for new runs.");}catch(error){$("#agents-error").textContent=error.status===409?"Settings changed elsewhere. Close and reopen this editor to review the latest version.":"Could not save. Choose an existing agent for each default role and check the field values.";}finally{$("#save-agents").disabled=false;}});
