export const columns = ["backlog", "ready", "active", "review", "done", "cancelled"];
export const liveRunStates = new Set(["queued", "admitted", "running", "blocked", "unknown", "cancel_requested", "paused", "awaiting_merge"]);
export const labels = { backlog: "Backlog", ready: "Ready", active: "Active", review: "Review", done: "Done", cancelled: "Canceled" };
export const priorities = { 0: "No priority", 1: "Urgent", 2: "High", 3: "Normal", 4: "Low" };

export function executionNotice(capabilities, connected = true, now = Date.now()) {
  const tick = capabilities.last_tick_at;
  const checkIn = Number.isSafeInteger(tick) && tick >= 0 && tick <= 8640000000000000 && tick <= now
    ? ` Last scheduler check-in: ${Math.floor((now - tick) / 1000)}s ago (${new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "long" }).format(tick)}).` : "";
  if (!connected) return "Connection interrupted. Displayed data may be stale; reconnect before making changes.";
  if (capabilities.execution_state === "reconciliation_required") return "Execution needs reconciliation after an uncertain dispatch. New runs are disabled until the recorded outcome is verified.";
  if (capabilities.execution_state === "unavailable") return "Task planning is available. Execution requires an approved worker connection.";
  if (capabilities.paused) return "Launches are paused. Current work may continue; use Stop on its task to request termination." + checkIn;
  if (capabilities.execution_state === "busy") return "The execution host is processing a run. New runs are disabled while this cycle is active.";
  if (capabilities.execution_state === "stale") return "Execution health is not verified. You can organize tasks; new runs are disabled until a fresh execution check completes. A scheduler check-in alone does not verify an active run." + checkIn;
  return null;
}

export function activityTitle(event) {
  const stages = { prepare: "Preparing checkout", coder: "Coder stage", qa: "QA stage", deliver: "Delivery stage", awaiting_merge: "Waiting for manual merge", admitted: "Run admitted", admission: "Run admitted", dispatch: "Agent started", running: "Agent working", qa_ready: "Implementation ready for QA", repair_ready: "Repair requested after QA", awaiting_acceptance: "Result ready for acceptance", merged: "Pull request merged", quality: "Review evidence recorded", dispatch_uncertain: "Execution needs reconciliation", receipt_missing: "Execution receipt missing", deadline_exceeded: "Execution deadline reached" };
  if (stages[event.stage]) return stages[event.stage];
  if (/^qa_admitted_\d+$/.test(event.stage || "")) return "QA started";
  if (/^coder_admitted_\d+$/.test(event.stage || "")) return "Implementation started";
  if (event.type === "run.updated" && event.status) return `Run ${String(event.status).replaceAll("_", " ")}`;
  return String(event.type || event.kind || "Activity recorded").replace(/[._]/g, " ");
}

export function executionReason(value) {
  const messages = {
    native_setup_blocked: "The coding agent could not finish project setup. Check the recorded setup failure before retrying.",
    native_validation_incomplete: "The agent did not complete the required project checks. This task has not passed validation.",
    native_validation_invalid: "The reported checks could not be matched to completed commands. Review the agent result before retrying.",
    native_auth_refresh_unknown: "Provider sign-in changed during the run. Reconcile access before continuing this repository.",
    native_cleanup_unknown: "Worker cleanup has not been confirmed. Reconcile this run before continuing the repository.",
    native_deadline_exceeded: "The coding-agent stage reached its deadline.",
    native_cancelled: "The coding-agent stage stopped before completion.",
    native_process_failed: "The coding agent exited before completion. Check its recorded failure before retrying.",
    native_result_invalid: "The agent returned a result that could not be verified.",
    native_turn_incomplete: "The coding agent did not finish its turn.",
    native_qa_source_changed: "QA changed source files. A clean independent review is required before delivery.",
    native_protected_source_path: "The implementation changed protected project metadata. Review those changes before continuing.",
    native_unignored_environment: "Setup files are included in the source changes. Keep local environments out of the implementation.",
    completion_persistence_failed:"A stage finished, but its result could not be saved. Check the run outcome before continuing.",
    qa_repair_queued:"QA requested changes. Coder will repair this same run before a fresh independent review.",
    qa_repairs_exhausted:"QA still requires changes after the allowed repairs. Review the findings before continuing this repository.",
    qa_feedback_missing:"QA rejected the implementation without actionable findings. Review the result before continuing.",
    qa_failed: "QA found changes that need attention.",
    driver_reported_failure: "A stage failed. Review its result before starting another run.",
    restart_requires_reconciliation: "Execution was interrupted. Check its recorded outcome.",
    deadline_exceeded: "The run reached its deadline.",
    driver_outcome_unknown: "The stage outcome needs reconciliation.",
    cleanup_unconfirmed: "Worker termination has not been confirmed.",
    cleanup_confirmed: "Worker termination and publication outcome were checked.",
    reconciled_without_delivery: "The outcome was checked; delivery did not complete.",
    manual_merge_veto: "The publication outcome needs review against the manual-merge policy.",
  };
  return messages[value] || (value ? "Execution needs attention. Check the recorded outcome." : null);
}

/** Queue availability alone is not evidence of active work. */
export function repositoryQueueStatus(project, queued = 0, launchesPaused = false) {
  if (project.failure) return "Needs attention";
  if (project.runtime_status === "preparing") return "Preparing checks";
  if (project.lane_run_id) return "Working";
  if (project.paused || launchesPaused) return "Paused";
  return queued > 0 ? "Queued" : "Idle";
}

export function activeRun(task, runs) {
  const latest = runs.filter(run => run.task_id === task.id).sort((a, b) => String(b.created_at || "").localeCompare(String(a.created_at || "")))[0];
  return latest && liveRunStates.has(latest.status) ? latest : null;
}

export function isArchived(task,runs=[]) { return Boolean(task.archived_at)&&!runs.some(run=>run.task_id===task.id&&liveRunStates.has(run.status)); }

export function taskStatus(task, runs) {
  if (activeRun(task, runs)?.status === "awaiting_merge") return "review";
  if (activeRun(task, runs)||task.archived_at&&!isArchived(task,runs)) return "active";
  return columns.includes(task.status) ? task.status : "backlog";
}

export function attentionReason(task, runs) {
  if(isArchived(task,runs))return null;
  if(task.archived_at)return "Archived task has unresolved execution";
  const run = activeRun(task, runs);
  if (task.blocked_reason) return task.blocked_reason;
  if(task.dependency_blockers?.length && task.status==="ready")return dependencyReason(task);
  if (run?.status === "awaiting_merge") return "Waiting for manual merge";
  if (run?.status === "paused") return "Launches paused";
  if (run?.status === "unknown") return "Execution needs reconciliation";
  if (run?.status === "blocked") return run.reason || run.blocked_reason || "Run needs attention";
  if (run?.status === "cancel_requested") return "Waiting for stop confirmation";
  if (task.status === "review") return "Result ready to review";
  const latest = runs.find(item => item.task_id === task.id);
  if (!["done","cancelled"].includes(task.status) && ["failed", "blocked"].includes(latest?.status)) return executionReason(latest.reason || latest.blocked_reason) || "Run needs attention";
  return null;
}

export function sortQueuedTasks(tasks) {
  const rank = task => task.priority === 0 ? 5 : (Number.isInteger(task.priority) && task.priority >= 1 && task.priority <= 4 ? task.priority : 3);
  const queuedAt = task => String(task.ready_at ?? task.created_at ?? "");
  return tasks.slice().sort((a, b) => rank(a) - rank(b) || queuedAt(a).localeCompare(queuedAt(b)) || String(a.id).localeCompare(String(b.id)));
}

export function filterTasks(tasks, { search = "", project = "", owner = "", task_type="", label="", scope = "active", runs = [] } = {}) {
  const needle = search.trim().toLowerCase();
  return tasks.filter(task => (scope === "archive" ? isArchived(task,runs) : !isArchived(task,runs)) && (!project || task.project === project) && (!owner || task.owner === owner) && (!task_type || (task.task_type||"feature")===task_type) && (!label || task.labels?.includes(label)) && (!needle || [task.id, task.source_id, task.title, task.objective, task.project, task.owner, ...(task.labels||[])].some(value => String(value || "").toLowerCase().includes(needle))));
}

export function safeWebUrl(value) {
  try {
    const url = new URL(value);
    return url.protocol === "https:" && !url.username && !url.password ? url.href : null;
  } catch { return null; }
}

export function safeArtifactUrl(value, origin) {
  const candidate = safeWebUrl(value);
  if (!candidate) return null;
  const url = new URL(candidate);
  return url.host === "github.com" || (origin && url.origin === origin) ? candidate : null;
}

export function taskPayload(form) {
  const lines = value => String(value || "").split(/\r?\n/).map(line => line.trim()).filter(Boolean);
  return {
    task_type: form.task_type||"feature", labels: String(form.labels||"").split(",").map(s=>s.trim()).filter(Boolean), dependencies: Array.isArray(form.dependencies)?form.dependencies:String(form.dependencies||"").split(/[,\n]/).map(s=>s.trim()).filter(Boolean),
    qa_policy: form.qa_policy || defaultQaPolicy(form.task_type),
    title: String(form.title || "").trim(), objective: String(form.objective || "").trim(),
    acceptance_criteria: lines(form.acceptance_criteria), context: lines(form.context),
    ...(form.project_id !== undefined ? {project_id:form.workflow === "engineering" ? String(form.project_id||"").trim()||null:null}:{}),
    project: String(form.project || "").trim() || null, owner: String(form.owner || "").trim() || null,
    ...(form.coding_agent_id !== undefined ? {coding_agent_id: form.workflow === "engineering" ? String(form.coding_agent_id || "").trim() || null : null} : {}),
    criticality: String(form.criticality || "").trim() || null,
    priority: Number(form.priority ?? 3), status: form.status || "backlog", workflow: form.workflow || "manual",
    repository: form.workflow === "engineering" ? String(form.repository || "").trim() || null : null,
    merge_policy: form.workflow === "engineering" ? form.merge_policy || "auto" : "manual",
    ...(form.automation_policy !== undefined ? { automation_policy: form.workflow === 'engineering' ? form.automation_policy : 'manual' } : {}),
    github_issue_url: String(form.github_issue_url || "").trim() || null,
    github_pull_request_url: String(form.github_pull_request_url || "").trim() || null,
    blocked_reason: String(form.blocked_reason || "").trim() || null,
  };
}

/** Accept only the configured private sign-in route, never arbitrary redirects. */
export function privateWorkspaceLink(value) {
  if (typeof value !== "string") return null;
  try {
    const url = new URL(value);
    if (url.protocol !== "https:" || !/^[a-z0-9-]+(?:\.[a-z0-9-]+)*\.ts\.net$/i.test(url.hostname) || url.username || url.password || url.pathname !== "/" || url.search || url.hash !== "#work") return null;
    return url.href;
  } catch { return null; }
}

/** Text references permit http(s), but never credentials, controls or HTML. */
export function safeDescriptionUrl(value) {
  if (typeof value !== "string" || /[\s\\\x00-\x1f\x7f]/.test(value)) return null;
  try {
    const url = new URL(value);
    return ["https:", "http:"].includes(url.protocol) && !url.username && !url.password ? url.href : null;
  } catch { return null; }
}
export function safeGithubTaskUrl(value, kind) {
  if (typeof value !== "string" || !["issues", "pull"].includes(kind)) return null;
  const pattern = new RegExp(`^https://github\\.com/[A-Za-z0-9][A-Za-z0-9-]{0,38}/[A-Za-z0-9._-]{1,100}/${kind}/[1-9][0-9]*$`);
  return pattern.test(value) && Number.isSafeInteger(Number(value.split("/").at(-1))) && safeWebUrl(value) === value ? value : null;
}

/** Small text tokenizer, not a Markdown/HTML interpreter. Nonlinks stay exact. */
export function descriptionParts(value) {
  const input = String(value ?? ""), parts = [];
  const pattern = /\[([^\[\]\r\n]+)\]\(((?:[^\s<>()]|\([^\s<>()]*\))*)\)|\bhttps?:\/\/[^\s<>"`]+/gi;
  let cursor = 0;
  for (const match of input.matchAll(pattern)) {
    if (match.index > cursor) parts.push({ text: input.slice(cursor, match.index) });
    let raw = match[0], label = match[1], target = match[2], suffix = "";
    if (label === undefined) {
      // Sentence punctuation and unmatched closing delimiters are ordinary text.
      const counts = {};
      for (const character of raw) counts[character] = (counts[character] || 0) + 1;
      let end = raw.length;
      while (end) {
        const character = raw[end - 1], opener = { ")": "(", "]": "[", "}": "{" }[character];
        if (!/[.,;:!?]/.test(character) && !(opener && counts[character] > (counts[opener] || 0))) break;
        counts[character]--; end--;
      }
      suffix = raw.slice(end); raw = raw.slice(0, end);
      target = raw; label = raw;
    }
    const href = safeDescriptionUrl(target);
    parts.push(href ? { text: label, href } : { text: raw });
    if (suffix) parts.push({ text: suffix });
    cursor = match.index + match[0].length;
  }
  if (cursor < input.length) parts.push({ text: input.slice(cursor) });
  return parts;
}


/** Display-only summaries never participate in task or Run authorization. */
export function latestRun(task, runs) {
  return runs.filter(run => run.task_id === task.id).slice().sort((a,b) => String(b.created_at || "").localeCompare(String(a.created_at || "")) || String(b.id || "").localeCompare(String(a.id || "")))[0] ?? null;
}
export function taskBlocker(task, runs) {
  if (task.blocked_reason) return { label: "Blocked", reason: task.blocked_reason, historical: false };
  if(task.dependency_blockers?.length && task.status==="ready")return {label:"Waiting",reason:dependencyReason(task),historical:false};
  const run = latestRun(task, runs);
  if (run?.status === "blocked" && !["done","cancelled"].includes(task.status)) return { label: "Blocked", reason: executionReason(run.reason || run.blocked_reason), historical: false };
  if (task.source_history?.status_name?.trim().toLowerCase() === "blocked" || task.source_history?.labels?.some(label => typeof label === "string" && label.trim().toLowerCase() === "blocked")) return { label: "Imported Blocked", reason: "Historical source label; current execution policy is unchanged.", historical: true };
  return null;
}
export function activityTime(value, now = Date.now()) {
  if (typeof value !== "string" || !/^\d{4}-\d{2}-\d{2}T/.test(value)) return null;
  const at = Date.parse(value);
  if (!Number.isFinite(at) || new Date(at).toISOString() !== value || at > now || at < 0) return null;
  const seconds = Math.floor((now - at) / 1000), minutes = Math.floor(seconds / 60), hours = Math.floor(minutes / 60), days = Math.floor(hours / 24);
  return { iso: value, relative: seconds < 60 ? "just now" : minutes < 60 ? `${minutes}m ago` : hours < 24 ? `${hours}h ago` : `${days}d ago`, exact: new Date(at).toLocaleString(undefined, { year:"numeric", month:"short", day:"numeric", hour:"2-digit", minute:"2-digit", second:"2-digit", timeZoneName:"short" }) + ` (${value})` };
}

export function automationLabel(task, latestRun) {
  if (task.automation_policy === "ready_to_done" && task.automation_revision === task.revision) return task.repository ? "Queued for automatic processing" : "Enrolled for this revision; requires activated controller policy";
  if (Number.isSafeInteger(latestRun?.automation?.revision) && latestRun.automation.revision > 0) return `Automatically picked up from revision ${latestRun.automation.revision}`;
  return "Explicit Run";
}

export function kanbanMovePayload(task, target, runs = []) {
  if (task.archived_at || activeRun(task, runs) || !["backlog", "ready"].includes(task.status) || !["backlog", "ready"].includes(target) || task.status === target) return null;
  if (target === "ready" && task.workflow === "engineering" && !task.repository && !task.project_id) return null;
  return { expected_revision: task.revision, status: target, automation_policy: target === "ready" && task.workflow === "engineering" ? "ready_to_done" : "manual" };
}

export function cardPriorityPayload(task, priority) {
  const enrolled = task.workflow === "engineering" && task.automation_policy === "ready_to_done" && task.automation_revision === task.revision;
  return { expected_revision: task.revision, priority: Number(priority), automation_policy: enrolled ? "ready_to_done" : "manual" };
}

export function readyTaskPayload(task) {
 return {status:"ready",expected_revision:task.revision,...((task.repository || task.project_id) && task.workflow === "engineering" && task.automation_policy === "ready_to_done" && (task.merge_policy === "auto" || task.repository) ? {automation_policy:"ready_to_done"} : {})};
}

/** Hidden processing controls must never enroll a planning-only task. */
export function initialAutomationPolicy(capabilities, editing = false, task = null) {
  if(editing && capabilities?.projects === true && capabilities?.ready_to_done === true && task?.workflow === "engineering" && ["manual","ready_to_done"].includes(task.automation_policy))return task.automation_policy;
  return !editing && capabilities?.ready_to_done === true ? "ready_to_done" : "manual";
}

export function qaBugLabel(bug) { return `${String(bug?.criticality || "").toUpperCase()} · ${bug?.path || ""}:${bug?.line || ""} — ${bug?.summary || ""}`; }

/** Collapse adjacent duplicate Run rows only; keep the durable audit untouched. */
export function coalesceActivity(events, displayedTime) {
 const rows=[];let previous=null;
 for(const event of events){
  const at=event.at || event.occurred_at;
  const key=event.run_id && ["run.created","run.updated"].includes(event.type) && at && Number.isFinite(Date.parse(at)) ? JSON.stringify([event.run_id,event.task_id,event.stage,event.reason,event.task_revision,event.cycle,event.repair_count,event.attempt_id,event.action_id,event.summary || activityTitle(event),displayedTime(at)]) : null;
  if(key===null || key!==previous)rows.push(event);
  previous=key;
 }
 return rows;
}

export function defaultQaPolicy(taskType){return taskType==="research"?"skip":"required";}
export function dependencyReason(task){return "Waiting for: "+(task.dependency_blockers||[]).map(d=>d.title||d.id).join(", ");}

export function planSelection(proposals, edits) {return proposals.filter(p=>edits.get(p.proposal_id)?.selected).map(p=>({proposal_id:p.proposal_id,qa_policy:edits.get(p.proposal_id).qa_policy||defaultQaPolicy(edits.get(p.proposal_id).task_type),proposal:{task_type:edits.get(p.proposal_id).task_type||"feature",labels:String(edits.get(p.proposal_id).labels||"").split(",").map(s=>s.trim()).filter(Boolean),depends_on:edits.get(p.proposal_id).depends_on||[],title:String(edits.get(p.proposal_id).title||"").trim(),objective:String(edits.get(p.proposal_id).objective||"").trim(),acceptance_criteria:String(edits.get(p.proposal_id).acceptance_criteria||"").split(/\r?\n/).map(s=>s.trim()).filter(Boolean),priority:Number(edits.get(p.proposal_id).priority)}}));}

/** Agent IDs select a registered runner; display owner names never select one. */
export function codingAgentSelection(config, task = null) {
  const seen = new Set();
  const agents = (Array.isArray(config?.agents) ? config.agents : []).filter(agent => {
    if (!agent || typeof agent.id !== "string" || !/^[a-z0-9_-]{1,40}$/.test(agent.id) || typeof agent.label !== "string" || !agent.label || seen.has(agent.id)) return false;
    seen.add(agent.id); return true;
  });
  const requested = task?.coding_agent_id || config?.default_coder || "";
  return {agents, selected: agents.some(agent => agent.id === requested) ? requested : "", unavailable: Boolean(task?.coding_agent_id && !agents.some(agent => agent.id === task.coding_agent_id))};
}
