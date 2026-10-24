use crate::{
    ApiError, App, Result,
    execution_config::{Approval, Program, now_ms, sha},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::Path, process::Stdio, sync::atomic::Ordering, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub schema_version: u32,
    pub action_id: String,
    pub run_id: String,
    pub action: String,
    pub repository: String,
    pub outcome: String,
    pub cleanup_complete: bool,
    pub workspace: Option<String>,
    pub base_sha: Option<String>,
    pub head_sha: Option<String>,
    pub base_ref: Option<String>,
    pub session_id: Option<String>,
    pub role: Option<String>,
    pub reviewed_sha: Option<String>,
    pub passed: Option<bool>,
    pub pull_request_url: Option<String>,
    pub merged: Option<bool>,
    pub merge_commit_sha: Option<String>,
    pub observed_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub findings: Option<Vec<crate::qa_feedback::Finding>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qa_completed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_of_scope_bugs: Option<Vec<crate::qa_feedback::OutOfScopeBug>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<crate::planning::Draft>,
}
fn error(code: &'static str) -> ApiError {
    ApiError::unavailable(code)
}
fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| error("execution_record_invalid"))
}
fn receipt_text(value: &Option<String>) -> Result<&str> {
    value
        .as_deref()
        .ok_or_else(|| error("driver_receipt_invalid"))
}

impl App {
    pub fn start_execution(&self) -> tokio::task::JoinHandle<()> {
        let app = self.clone();
        tokio::spawn(async move {
            let mut workers = tokio::task::JoinSet::new();
            let mut preparations = tokio::task::JoinSet::new();
            let mut planning = tokio::task::JoinSet::new();
            loop {
                let stopping = app.inner.stopping.load(Ordering::SeqCst);
                if stopping {
                    preparations.abort_all();
                    while preparations.join_next().await.is_some() {}
                    if let Ok(mut store) = app.inner.store.lock() {
                        if let Ok(projects) = store.projects() {
                            for p in projects {
                                if p["runtime_status"] == "preparing" {
                                    let _ = store.project_fault(
                                        p["id"].as_str().unwrap_or(""),
                                        "project_discovery_interrupted",
                                    );
                                }
                            }
                        }
                    }
                    if workers.is_empty() && planning.is_empty() {
                        break;
                    }
                }
                let notified = app.inner.wake.notified();
                if !stopping {
                    if planning.is_empty() {
                        if let Ok(Some(job)) = app.claim_plan() {
                            let planner = app.clone();
                            planning.spawn(async move {
                                planner.perform_plan(job).await;
                            });
                        }
                    }
                    if let Ok(Some((repository, project))) = app.next_repository_setup() {
                        let preparing = app.clone();
                        preparations.spawn(async move {
                            let factory = preparing.inner.factory.as_ref().unwrap();
                            let result = factory.prepare_repository(&repository, &project).await;
                            if let Ok(mut store) = preparing.inner.store.lock() {
                                match result {
                                    Ok(mut p) => {
                                        if let Ok(current) = crate::projects::get(
                                            &store.db,
                                            project["id"].as_str().unwrap(),
                                        ) {
                                            p["paused"] = current["paused"].clone();
                                            p["revision"] = json!(
                                                current["revision"].as_u64().unwrap_or(0) + 1
                                            );
                                        }
                                        if let Err(e) = store.project_update(&p) {
                                            let _ = store.project_fault(
                                                project["id"].as_str().unwrap(),
                                                e.code(),
                                            );
                                        }
                                    }
                                    Err(e) => {
                                        let _ = store.project_fault(
                                            project["id"].as_str().unwrap(),
                                            e.code(),
                                        );
                                    }
                                }
                            }
                            preparing.changed();
                        });
                        app.changed();
                        continue;
                    }
                    let next = app
                        .inner
                        .store
                        .lock()
                        .map_err(|_| ApiError::internal())
                        .and_then(|mut s| s.next_attempt());
                    if let Ok(Some((run, attempt))) = next {
                        let app = app.clone();
                        workers.spawn(async move {
                            let result = perform(&app, &run, &attempt).await;
                            let (next, receipt) = match result {
                                Ok(pair) => pair,
                                Err(code) => {
                                    let mut unknown = run.clone();
                                    unknown["status"] = json!("unknown");
                                    unknown["reason"] = json!(code.code());
                                    unknown["diagnostic"] =
                                        code.diagnostic().map(|v| v.json()).unwrap_or(Value::Null);
                                    (unknown, None)
                                }
                            };
                            if let Ok(mut store) = app.inner.store.lock() {
                                if store
                                    .finish_attempt(
                                        field(&run, "id").unwrap_or(""),
                                        field(&attempt, "id").unwrap_or(""),
                                        next,
                                        receipt,
                                    )
                                    .is_err()
                                {
                                    // The completion transaction rolled back. Keep the last
                                    // durable Run/attempt and lane; never retry the effect or
                                    // treat uncommitted QA/bug tickets as accepted evidence.
                                    let mut unknown = run.clone();
                                    unknown["status"] = json!("unknown");
                                    unknown["reason"] = json!("completion_persistence_failed");
                                    if store
                                        .finish_attempt(
                                            field(&run, "id").unwrap_or(""),
                                            field(&attempt, "id").unwrap_or(""),
                                            unknown,
                                            None,
                                        )
                                        .is_err()
                                    {
                                        // If storage itself is unavailable, stop all launches.
                                        // Startup already treats unfinished intents as unknown.
                                        app.shutdown();
                                    }
                                }
                            }
                            app.changed();
                        });
                        continue;
                    }
                    if app.pick_project().unwrap_or(false) || app.pick_automatic().unwrap_or(false)
                    {
                        app.changed();
                        continue;
                    }
                }
                tokio::select! {_=notified=>{},_=workers.join_next(),if !workers.is_empty()=>{},_=preparations.join_next(),if !preparations.is_empty()=>{},_=planning.join_next(),if !planning.is_empty()=>{}}
            }
        })
    }
    fn next_repository_setup(&self) -> Result<Option<(String, Value)>> {
        let Some(factory) = &self.inner.factory else {
            return Ok(None);
        };
        factory.verify()?;
        let mut store = self.inner.store.lock().map_err(|_| ApiError::internal())?;
        let projects = store.projects()?;
        if projects
            .iter()
            .filter(|p| p["runtime_status"] == "preparing")
            .count()
            >= 2
        {
            return Ok(None);
        }
        let snapshot = store.snapshot()?;
        let mut tasks = snapshot["tasks"].as_array().cloned().unwrap_or_default();
        tasks.sort_by_key(|t| {
            (
                if t["automation_policy"] == "ready_to_done"
                    && t["automation_revision"] == t["revision"]
                {
                    0
                } else {
                    1
                },
                match t["priority"].as_u64().unwrap_or(3) {
                    0 => 5,
                    n => n,
                },
                t["ready_at"].as_str().unwrap_or("").to_string(),
                t["id"].as_str().unwrap_or("").to_string(),
            )
        });
        let mut seen = std::collections::HashSet::new();
        for task in tasks {
            if task["workflow"] != "engineering"
                || task["status"] != "ready"
                || !task["blocked_reason"].is_null()
                || !task["archived_at"].is_null()
                || task["project_id"].is_string()
                || !crate::tickets::blockers(&store.db, &task)?.is_empty()
            {
                continue;
            }
            let Some(repository) = task["repository"]
                .as_str()
                .filter(|r| crate::store::valid_repository(r))
            else {
                continue;
            };
            if !seen.insert(repository.to_lowercase()) {
                continue;
            }
            let key = format!("{}:{}", task["id"].as_str().unwrap_or(""), task["revision"]);
            let existing = crate::projects::by_repository(&store.db, repository)?;
            if existing.as_ref().is_some_and(|p| {
                p["paused"] == true
                    || !p["lane_run_id"].is_null()
                    || p["runtime_status"] == "preparing"
                    || p["checked_task_revision"] == key
            }) {
                continue;
            }
            let id = uuid::Uuid::new_v4().to_string();
            let mut p=existing.unwrap_or_else(||json!({"id":id,"revision":1,"name":repository,"repository":repository,"repository_id":null,"default_branch":null,"local_path":format!("repository:{}",repository.to_lowercase()),"profile":null,"paused":false,"failure":null,"lane_run_id":null,"created_at":crate::store::now()}));
            p["runtime_status"] = json!("preparing");
            p["validation_summary"] = json!("Discovering checks and preparing dependencies");
            p["checked_task_revision"] = json!(key);
            p["updated_at"] = json!(crate::store::now());
            store.project_update(&p)?;
            return Ok(Some((repository.into(), p)));
        }
        Ok(None)
    }
    fn pick_project(&self) -> Result<bool> {
        let Some(factory) = &self.inner.factory else {
            return Ok(false);
        };
        factory.verify()?;
        let mut store = self.inner.store.lock().map_err(|_| ApiError::internal())?;
        if store.execution_state()?["paused"] != false {
            return Ok(false);
        }
        let projects = store.projects()?;
        let snapshot = store.snapshot()?;
        let mut tasks = snapshot["tasks"].as_array().cloned().unwrap_or_default();
        tasks.sort_by_key(|t| {
            (
                match t["priority"].as_u64().unwrap_or(3) {
                    0 => 5,
                    n => n,
                },
                t["ready_at"]
                    .as_str()
                    .or_else(|| t["created_at"].as_str())
                    .unwrap_or("")
                    .to_string(),
                t["id"].as_str().unwrap_or("").to_string(),
            )
        });
        let mut faulted = std::collections::HashSet::new();
        let mut changed = false;
        for task in tasks {
            let Some(project) = projects.iter().find(|p| {
                (p["id"] == task["project_id"]
                    || p["repository"].as_str().is_some_and(|r| {
                        task["repository"]
                            .as_str()
                            .is_some_and(|t| r.eq_ignore_ascii_case(t))
                    }))
                    && p["runtime_status"] == "configured"
                    && p["paused"] == false
                    && p["lane_run_id"].is_null()
                    && !faulted.contains(p["id"].as_str().unwrap_or(""))
            }) else {
                continue;
            };
            if task["status"] != "ready"
                || task["workflow"] != "engineering"
                || task["automation_policy"] != "ready_to_done"
                || task["automation_revision"] != task["revision"]
                || !task["blocked_reason"].is_null()
                || !task["archived_at"].is_null()
                || !crate::tickets::blockers(&store.db, &task)?.is_empty()
            {
                continue;
            }
            if task["project_id"].is_null()
                && project["checked_task_revision"]
                    != format!("{}:{}", task["id"].as_str().unwrap_or(""), task["revision"])
            {
                continue;
            }
            let registry = crate::system::get(&store.db)?;
            let approval = match factory.task_approval(project, &task, &registry) {
                Ok(a) => a,
                Err(e) if e.code() == "agent_unavailable" => continue,
                Err(e) => {
                    let id = project["id"].as_str().unwrap_or("");
                    store.project_fault(id, e.code())?;
                    faulted.insert(id.to_string());
                    changed = true;
                    continue;
                }
            };
            let key = format!(
                "workboard-automatic:{}:{}",
                task["id"].as_str().unwrap_or(""),
                task["revision"]
            );
            let result = store.execution_command_guarded(
                &key,
                &key,
                "run",
                task["id"].as_str(),
                json!({"expected_revision":task["revision"]}),
                crate::execution_store::Admission {
                    approval: Some(&approval),
                    ..Default::default()
                },
            );
            match result {
                Ok(_) => return Ok(true),
                Err(e)
                    if matches!(
                        e.code(),
                        "automatic_run_already_consumed" | "active_run_exists"
                    ) =>
                {
                    continue;
                }
                Err(e) => {
                    let id = project["id"].as_str().unwrap_or("");
                    store.project_fault(id, e.code())?;
                    faulted.insert(id.to_string());
                    changed = true;
                }
            }
        }
        Ok(changed)
    }
    fn pick_automatic(&self) -> Result<bool> {
        let Some(activation) = &self.inner.automation else {
            return Ok(false);
        };
        let Some(approval) = &self.inner.approval else {
            return Ok(false);
        };
        activation.verify(approval)?;
        self.assert_ownership(activation.repository())?;
        let mut store = self.inner.store.lock().map_err(|_| ApiError::internal())?;
        let state = store.execution_state()?;
        if state["paused"] != false || !state["lane_run_id"].is_null() {
            return Ok(false);
        }
        let snapshot = store.snapshot()?;
        for task in snapshot["tasks"]
            .as_array()
            .ok_or_else(ApiError::internal)?
        {
            if !activation.eligible(task) || !crate::tickets::blockers(&store.db, task)?.is_empty()
            {
                continue;
            }
            let key = format!(
                "workboard-automatic:{}:{}",
                task["id"].as_str().unwrap_or(""),
                task["revision"]
            );
            let body = json!({"expected_revision":task["revision"]});
            let result = store.execution_command_guarded(
                &key,
                &key,
                "run",
                task["id"].as_str(),
                body,
                crate::execution_store::Admission {
                    approval: Some(approval),
                    ownership: self.inner.ownership.as_ref(),
                    automatic: Some(activation),
                    repository_queue: false,
                    coding_agents: None,
                    agent_registry: None,
                },
            );
            match result {
                Ok(_) => return Ok(true),
                Err(e) if e.code() == "automatic_run_already_consumed" => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(false)
    }
}
async fn perform(app: &App, run: &Value, attempt: &Value) -> Result<(Value, Option<Value>)> {
    let owned_approval = app.run_approval(run)?;
    let approval = &owned_approval;
    let action = field(attempt, "action")?;
    let observation = matches!(action, "observe" | "cancel");
    approval.verify(field(run, "approval_sha256")?, observation)?;
    if !observation {
        app.assert_ownership(field(run, "repository")?)?;
        app.assert_automatic_run(run)?;
    }
    if !observation && now_ms() >= run["deadline_ms"].as_i64().unwrap_or(0) {
        return Err(error("deadline_exceeded"));
    }
    if !matches!(action, "prepare" | "cancel" | "observe") {
        verify_workspace(approval, run, false).await?;
    }
    if matches!(action, "cancel" | "observe") {
        verify_location(approval, run)?;
    }
    let mut request = json!({"schema_version":1,"action":action,"action_id":attempt["id"],"run_id":run["id"],"repository":run["repository"],"task_snapshot":run["task_snapshot"],"deadline_ms":run["deadline_ms"],"stage_deadline_ms":if observation {now_ms()+approval.policy.max_stage_seconds as i64*1000} else {run["deadline_ms"].as_i64().unwrap_or(0).min(now_ms()+approval.policy.max_stage_seconds as i64*1000)},"workspace":run["workspace"],"base_sha":run["base_sha"],"head_sha":run["head_sha"],"base_ref":run["base_ref"],"qa_receipt":run["qa_receipt"],"merge_policy":run["task_snapshot"]["merge_policy"],"observed_action_id":run["last_effect_attempt"],"approval_sha256":run["approval_sha256"],"adapter_policy":approval.policy.adapter_policy});
    if run["task_snapshot"]["qa_policy"] == "skip" {
        request["coder_receipt"] = run["coder_receipt"].clone();
    }
    if action == "coder" && !run["repair_context"].is_null() {
        request["repair_context"] = run["repair_context"].clone();
    }
    let program = if attempt["driver"] == "worker" {
        &approval.policy.worker
    } else {
        &approval.policy.publisher
    };
    let receipt = match driver(app, approval, program, run, action, &request).await? {
        DriverOutcome::Completed(receipt) => *receipt,
        outcome => {
            // These outcomes are known before spawn, never inferred from a
            // missing receipt. Only prepare has no prior effect to preserve.
            let mut deferred = run.clone();
            if action == "prepare" {
                deferred["last_effect_attempt"] = Value::Null;
            }
            match outcome {
                DriverOutcome::StoppedBeforeSpawn => deferred["status"] = json!("cancel_requested"),
                DriverOutcome::ShutdownBeforeSpawn => {
                    deferred["status"] = json!("unknown");
                    deferred["reason"] = json!("driver_interrupted");
                }
                _ => {}
            }
            return Ok((deferred, None));
        }
    };
    if receipt.schema_version != 1
        || receipt.action_id != field(attempt, "id")?
        || receipt.run_id != field(run, "id")?
        || receipt.action != action
        || receipt.repository != field(run, "repository")?
        || !["ok", "failed", "unknown"].contains(&receipt.outcome.as_str())
        || !receipt.cleanup_complete
    {
        return Err(error("driver_receipt_invalid"));
    }
    let mut next = run.clone();
    next["reason"] = Value::Null;
    if action != "qa"
        && receipt
            .out_of_scope_bugs
            .as_ref()
            .is_some_and(|b| !b.is_empty())
    {
        return Err(error("driver_receipt_invalid"));
    }
    if receipt.outcome == "unknown" {
        return Err(error("driver_outcome_unknown"));
    }
    if receipt.outcome == "failed" && matches!(action, "cancel" | "observe") {
        return Err(error("cleanup_unconfirmed"));
    }
    if receipt.outcome == "failed" && action != "qa" {
        next["status"] = json!("failed");
        next["reason"] = json!("driver_reported_failure");
        return Ok((next, Some(safe_receipt(&receipt))));
    }
    match action {
        "prepare" => {
            if receipt_text(&receipt.workspace)? != field(run, "workspace")?
                || !sha(receipt_text(&receipt.base_sha)?)
                || receipt.head_sha != receipt.base_sha
            {
                return Err(error("prepare_receipt_invalid"));
            }
            let base_ref = receipt_text(&receipt.base_ref)?;
            if !base_ref.starts_with("refs/remotes/origin/")
                || base_ref.len() > 160
                || base_ref.contains("..")
                || !base_ref
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/_-.".contains(&b))
            {
                return Err(error("prepare_receipt_invalid"));
            }
            next["base_sha"] = json!(receipt.base_sha);
            next["head_sha"] = json!(receipt.head_sha);
            next["base_ref"] = json!(base_ref);
            verify_workspace(approval, &next, false).await?;
            if git(
                approval,
                field(&next, "workspace")?,
                &["rev-parse", "--verify", base_ref],
            )
            .await?
            .trim()
                != field(&next, "base_sha")?
            {
                return Err(error("base_sha_invalid"));
            }
            next["stage"] = json!("coder");
            next["status"] = json!("running");
        }
        "coder" => {
            if run["task_snapshot"]["qa_policy"] == "skip" && receipt.passed != Some(true) {
                return Err(error("coder_receipt_invalid"));
            }
            validate_identity(&receipt, "coder")?;
            if receipt.workspace.as_deref() != run["workspace"].as_str()
                || receipt.base_sha.as_deref() != run["base_sha"].as_str()
                || !sha(receipt_text(&receipt.head_sha)?)
                || receipt.head_sha == receipt.base_sha
            {
                return Err(error("coder_receipt_invalid"));
            }
            record_session(&mut next, &receipt)?;
            if receipt.head_sha.as_deref() == run["head_sha"].as_str() {
                return Err(error("coder_receipt_invalid"));
            }
            next["head_sha"] = json!(receipt.head_sha);
            next["coder_session"] = json!(receipt.session_id);
            next["qa_receipt"] = Value::Null;
            verify_workspace(approval, &next, false).await?;
            git(
                approval,
                field(run, "workspace")?,
                &[
                    "merge-base",
                    "--is-ancestor",
                    field(run, "head_sha")?,
                    field(&next, "head_sha")?,
                ],
            )
            .await?;
            next["coder_receipt"] =
                serde_json::to_value(&receipt).map_err(|_| error("coder_receipt_invalid"))?;
            next["stage"] = json!(if run["task_snapshot"]["qa_policy"] == "skip" {
                "deliver"
            } else {
                "qa"
            });
        }
        "qa" => {
            validate_identity(&receipt, "qa")?;
            if receipt.session_id.as_deref() == run["coder_session"].as_str()
                || receipt.reviewed_sha.as_deref() != run["head_sha"].as_str()
                || receipt.head_sha.as_deref() != run["head_sha"].as_str()
                || receipt.passed.is_none()
            {
                return Err(error("qa_receipt_invalid"));
            }
            verify_workspace(approval, run, false).await?;
            next["qa_receipt"] =
                serde_json::to_value(&receipt).map_err(|_| error("qa_receipt_invalid"))?;
            record_session(&mut next, &receipt)?;
            if let Some(findings) = &receipt.findings {
                crate::qa_feedback::findings(findings)?;
            }
            if receipt.qa_completed == Some(false)
                || (receipt.passed == Some(true)
                    && receipt.findings.as_ref().is_some_and(|f| !f.is_empty()))
            {
                return Err(error("qa_receipt_invalid"));
            }
            let bugs = receipt.out_of_scope_bugs.clone().unwrap_or_default();
            crate::qa_feedback::out_of_scope_bugs(&bugs, field(run, "base_sha")?)?;
            if !bugs.is_empty()
                && (receipt.qa_completed != Some(true)
                    || (receipt.passed == Some(false)
                        && receipt.findings.as_ref().is_none_or(|f| f.is_empty())))
            {
                return Err(error("qa_receipt_invalid"));
            }
            let findings = receipt.findings.clone().unwrap_or_default();
            let cycle = run["repair_count"].as_u64().unwrap_or(0);
            let history = json!({"cycle":cycle,"reviewed_sha":receipt.reviewed_sha,"coder_session_id":run["coder_session"],"qa_session_id":receipt.session_id,"qa_action_id":receipt.action_id,"passed":receipt.passed,"findings":findings,"out_of_scope_bugs":bugs});
            if !next["review_history"].is_array() {
                next["review_history"] = json!([]);
            }
            next["review_history"].as_array_mut().unwrap().push(history);
            if receipt.passed == Some(false) {
                if findings.is_empty() {
                    next["status"] = json!("failed");
                    next["reason"] = json!(if receipt.findings.is_some() {
                        "qa_feedback_missing"
                    } else {
                        "qa_failed"
                    });
                } else if receipt.qa_completed != Some(true) {
                    return Err(error("qa_receipt_invalid"));
                } else if now_ms() >= run["deadline_ms"].as_i64().unwrap_or(0) {
                    next["status"] = json!("failed");
                    next["reason"] = json!("deadline_exceeded");
                } else if cycle >= run["max_qa_repairs"].as_u64().unwrap_or(0) {
                    next["status"] = json!("failed");
                    next["reason"] = json!("qa_repairs_exhausted");
                } else {
                    next["repair_count"] = json!(cycle + 1);
                    next["stage"] = json!("coder");
                    next["reason"] = json!("qa_repair_queued");
                    next["repair_context"] = json!(crate::qa_feedback::Repair {
                        cycle: (cycle + 1) as u8,
                        reviewed_sha: field(run, "head_sha")?.into(),
                        qa_action_id: receipt.action_id.clone(),
                        qa_session_id: receipt.session_id.clone().unwrap(),
                        findings
                    });
                }
            } else if receipt.outcome == "ok" {
                next["repair_context"] = Value::Null;
                next["stage"] = json!("deliver");
            } else {
                return Err(error("qa_receipt_invalid"));
            }
        }
        "deliver" => {
            verify_qa(run)?;
            verify_pr(&receipt, run)?;
            if run["task_snapshot"]["merge_policy"] == "manual" && receipt.merged != Some(false) {
                return Err(error("manual_merge_veto"));
            }
            next["pull_request_url"] = json!(receipt.pull_request_url);
            if receipt.merged == Some(true) {
                verify_merge(approval, run, &receipt).await?;
                next["merge_commit_sha"] = json!(receipt.merge_commit_sha);
                next["status"] = json!("succeeded");
                next["stage"] = json!("merged");
            } else if receipt.merged == Some(false) {
                next["status"] = json!("awaiting_merge");
                next["stage"] = json!("awaiting_merge");
            } else {
                return Err(error("delivery_receipt_invalid"));
            }
        }
        "cancel" => {
            if run["last_effect_attempt"].is_null() {
                // No effect-bearing driver ever spawned. Confirmed worker
                // cleanup is sufficient; no prepared publisher state exists.
                next["status"] = json!("cancelled");
                next["reason"] = json!("cleanup_confirmed");
            } else {
                next["stage"] = json!("observe_cancel");
                next["status"] = json!("running");
                next["cancel_cleanup"] = json!(true);
            }
        }
        "observe" => {
            if receipt.observed_action_id.as_deref() != run["last_effect_attempt"].as_str() {
                return Err(error("observation_receipt_invalid"));
            }
            if receipt.merged == Some(true) {
                verify_qa(run)?;
                verify_pr(&receipt, run)?;
                verify_merge(approval, run, &receipt).await?;
                next["pull_request_url"] = json!(receipt.pull_request_url);
                next["merge_commit_sha"] = json!(receipt.merge_commit_sha);
                next["status"] = json!("succeeded");
                next["stage"] = json!("merged");
            } else if receipt.merged == Some(false) {
                if run["stage"] == "observe_cancel" && run["cancel_cleanup"] == true {
                    next["status"] = json!("cancelled");
                    next["reason"] = json!("cleanup_confirmed");
                } else if run["stage"] == "awaiting_merge" {
                    next["status"] = json!("awaiting_merge");
                } else {
                    next["status"] = json!("failed");
                    next["reason"] = json!("reconciled_without_delivery");
                }
            } else {
                return Err(error("observation_receipt_invalid"));
            }
        }
        _ => return Err(error("execution_record_invalid")),
    }
    Ok((next, Some(safe_receipt(&receipt))))
}
fn validate_identity(receipt: &Receipt, role: &str) -> Result<()> {
    if receipt.role.as_deref() != Some(role)
        || !receipt.session_id.as_deref().is_some_and(|s| {
            (8..=128).contains(&s.len())
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-:.".contains(&b))
        })
    {
        return Err(error("worker_identity_invalid"));
    }
    Ok(())
}
fn record_session(run: &mut Value, receipt: &Receipt) -> Result<()> {
    let session = receipt
        .session_id
        .as_deref()
        .ok_or_else(|| error("worker_identity_invalid"))?;
    let mut seen = run["session_history"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if seen.iter().any(|v| v.as_str() == Some(session))
        || run["coder_session"].as_str() == Some(session)
    {
        return Err(error("worker_identity_reused"));
    }
    seen.push(json!(session));
    run["session_history"] = json!(seen);
    Ok(())
}
fn verify_qa(run: &Value) -> Result<()> {
    if run["task_snapshot"]["qa_policy"] == "skip" {
        let coder = &run["coder_receipt"];
        if coder["role"] != "coder"
            || coder["action"] != "coder"
            || coder["passed"] != true
            || coder["cleanup_complete"] != true
            || coder["head_sha"] != run["head_sha"]
            || coder["run_id"] != run["id"]
            || coder["repository"] != run["repository"]
        {
            return Err(error("coder_receipt_invalid"));
        }
        return Ok(());
    }
    let qa = &run["qa_receipt"];
    if qa["passed"] != true
        || qa["role"] != "qa"
        || qa["reviewed_sha"] != run["head_sha"]
        || qa["session_id"] == run["coder_session"]
        || qa["cleanup_complete"] != true
    {
        return Err(error("qa_receipt_invalid"));
    }
    Ok(())
}
fn verify_pr(receipt: &Receipt, run: &Value) -> Result<()> {
    let prefix = format!("https://github.com/{}/pull/", field(run, "repository")?);
    let suffix = receipt_text(&receipt.pull_request_url)?
        .strip_prefix(&prefix)
        .ok_or_else(|| error("delivery_receipt_invalid"))?;
    if suffix.is_empty()
        || suffix.starts_with('0')
        || suffix.len() > 20
        || !suffix.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(error("delivery_receipt_invalid"));
    }
    Ok(())
}
fn safe_receipt(receipt: &Receipt) -> Value {
    json!({"action":receipt.action,"action_id":receipt.action_id,"outcome":receipt.outcome,"cleanup_complete":receipt.cleanup_complete,"head_sha":receipt.head_sha,"reviewed_sha":receipt.reviewed_sha,"passed":receipt.passed,"merged":receipt.merged,"merge_commit_sha":receipt.merge_commit_sha,"findings":receipt.findings,"qa_completed":receipt.qa_completed,"out_of_scope_bugs":receipt.out_of_scope_bugs})
}
fn verify_location(approval: &Approval, run: &Value) -> Result<()> {
    let expected = approval.policy.work_root.join(field(run, "id")?);
    let path = Path::new(field(run, "workspace")?);
    if path != expected || (path.exists() && path.canonicalize().ok().as_ref() != Some(&expected)) {
        return Err(error("workspace_invalid"));
    }
    Ok(())
}
async fn verify_workspace(
    approval: &Approval,
    run: &Value,
    allow_changed_head: bool,
) -> Result<()> {
    let path = Path::new(field(run, "workspace")?);
    let expected = approval.policy.work_root.join(field(run, "id")?);
    if path != expected || path.canonicalize().ok().as_ref() != Some(&expected) || !path.is_dir() {
        return Err(error("workspace_invalid"));
    }
    let gitdir = path.join(".git");
    let meta = std::fs::symlink_metadata(&gitdir).map_err(|_| error("workspace_invalid"))?;
    if !meta.is_dir() || gitdir.canonicalize().ok().as_ref() != Some(&gitdir) {
        return Err(error("workspace_invalid"));
    }
    let config = gitdir.join("config");
    let configmeta = std::fs::symlink_metadata(&config).map_err(|_| error("workspace_invalid"))?;
    if !configmeta.is_file()
        || configmeta.len() > 16384
        || gitdir.join("objects/info/alternates").exists()
    {
        return Err(error("workspace_invalid"));
    }
    let configuration = git(
        approval,
        field(run, "workspace")?,
        &["config", "--no-includes", "--local", "--null", "--list"],
    )
    .await?;
    for entry in configuration.split('\0').filter(|s| !s.is_empty()) {
        let key = entry.split('\n').next().unwrap_or("");
        if !matches!(
            key,
            "core.repositoryformatversion"
                | "core.filemode"
                | "core.bare"
                | "core.logallrefupdates"
                | "core.ignorecase"
                | "core.precomposeunicode"
                | "remote.origin.url"
                | "remote.origin.fetch"
        ) && !(key.starts_with("branch.")
            && (key.ends_with(".remote") || key.ends_with(".merge")))
        {
            return Err(error("repository_config_unsafe"));
        }
    }
    let root = git(
        approval,
        field(run, "workspace")?,
        &["rev-parse", "--show-toplevel"],
    )
    .await?;
    if Path::new(root.trim()) != path {
        return Err(error("workspace_invalid"));
    }
    let common = git(
        approval,
        field(run, "workspace")?,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .await?;
    if !Path::new(common.trim())
        .canonicalize()
        .map_err(|_| error("workspace_invalid"))?
        .starts_with(&expected)
    {
        return Err(error("workspace_invalid"));
    }
    let origin = git(
        approval,
        field(run, "workspace")?,
        &["config", "--get", "remote.origin.url"],
    )
    .await?;
    let repo = field(run, "repository")?;
    if ![
        format!("https://github.com/{repo}"),
        format!("https://github.com/{repo}.git"),
        format!("git@github.com:{repo}.git"),
    ]
    .contains(&origin.trim().to_string())
    {
        return Err(error("repository_origin_invalid"));
    }
    if !git(
        approval,
        field(run, "workspace")?,
        &["status", "--porcelain", "--untracked-files=all"],
    )
    .await?
    .is_empty()
    {
        return Err(error("workspace_dirty"));
    }
    let head = git(
        approval,
        field(run, "workspace")?,
        &["rev-parse", "--verify", "HEAD"],
    )
    .await?;
    if !allow_changed_head && head.trim() != field(run, "head_sha")? {
        return Err(error("head_sha_invalid"));
    }
    if !run["base_sha"].is_null() {
        git(
            approval,
            field(run, "workspace")?,
            &[
                "merge-base",
                "--is-ancestor",
                field(run, "base_sha")?,
                head.trim(),
            ],
        )
        .await?;
    }
    Ok(())
}
async fn verify_merge(approval: &Approval, run: &Value, receipt: &Receipt) -> Result<()> {
    let merge = receipt_text(&receipt.merge_commit_sha)?;
    if !sha(merge) || receipt.head_sha.as_deref() != run["head_sha"].as_str() {
        return Err(error("delivery_receipt_invalid"));
    }
    verify_workspace(approval, run, false).await?;
    let parents = git(
        approval,
        field(run, "workspace")?,
        &["rev-list", "--parents", "-n", "1", merge],
    )
    .await?;
    let parents: Vec<_> = parents.split_whitespace().collect();
    if parents.len() != 3 || parents[0] != merge || parents[2] != field(run, "head_sha")? {
        return Err(error("merge_ancestry_invalid"));
    }
    git(
        approval,
        field(run, "workspace")?,
        &[
            "merge-base",
            "--is-ancestor",
            field(run, "base_sha")?,
            parents[1],
        ],
    )
    .await?;
    let base_ref = field(run, "base_ref")?;
    git(
        approval,
        field(run, "workspace")?,
        &["merge-base", "--is-ancestor", merge, base_ref],
    )
    .await?;
    Ok(())
}
pub(crate) async fn read_bounded<R: AsyncRead + Unpin>(
    mut stream: R,
    max: usize,
) -> std::result::Result<Vec<u8>, ()> {
    let mut output = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let n = stream.read(&mut chunk).await.map_err(|_| ())?;
        if n == 0 {
            return Ok(output);
        }
        if output.len() + n > max {
            return Err(());
        }
        output.extend_from_slice(&chunk[..n]);
    }
}
enum DriverOutcome {
    Completed(Box<Receipt>),
    PausedBeforeSpawn,
    StoppedBeforeSpawn,
    ShutdownBeforeSpawn,
}
async fn driver(
    app: &App,
    approval: &Approval,
    program: &Program,
    run: &Value,
    action: &str,
    request: &Value,
) -> Result<DriverOutcome> {
    approval.verify(
        field(run, "approval_sha256")?,
        matches!(action, "observe" | "cancel"),
    )?;
    let mut command = Command::new(&program.executable);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .current_dir(&approval.policy.work_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for name in &program.credential_env_names {
        let value = std::env::var_os(name).ok_or_else(|| error("execution_credential_missing"))?;
        command.env(name, value);
    }
    #[cfg(unix)]
    {
        command.process_group(0);
    }
    let mut child = {
        // Serialize the final launch decision with operator pause/Stop writes.
        // No await occurs while held, and acknowledged controls cannot be
        // overtaken by a driver still completing asynchronous Git preflight.
        let store = app.inner.store.lock().map_err(|_| ApiError::internal())?;
        if app.inner.stopping.load(Ordering::SeqCst) {
            return Ok(DriverOutcome::ShutdownBeforeSpawn);
        }
        if !matches!(action, "cancel" | "observe")
            && store.current_run(field(run, "id")?)?["status"] == "cancel_requested"
        {
            return Ok(DriverOutcome::StoppedBeforeSpawn);
        }
        let project_paused = run["project_id"]
            .as_str()
            .map(|id| crate::projects::get(&store.db, id).map(|p| p["paused"] != false))
            .transpose()?
            .unwrap_or(false);
        if !matches!(action, "cancel" | "observe")
            && (store.execution_state()?["paused"] == true || project_paused)
        {
            return Ok(DriverOutcome::PausedBeforeSpawn);
        }
        if !matches!(action, "cancel" | "observe") {
            app.assert_ownership(field(run, "repository")?)?;
            app.assert_automatic_run(run)?;
        }
        command.spawn().map_err(|_| error("driver_spawn_failed"))?
    };
    let pid = child.id().ok_or_else(|| error("driver_spawn_failed"))?;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| error("driver_transport_failed"))?;
    let output = child
        .stdout
        .take()
        .ok_or_else(|| error("driver_transport_failed"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| error("driver_transport_failed"))?;
    let bytes = request.to_string().into_bytes();
    let max = approval.policy.max_output_bytes;
    let remaining = if matches!(action, "observe" | "cancel") {
        approval.policy.max_stage_seconds * 1000
    } else {
        run["deadline_ms"]
            .as_i64()
            .unwrap_or(0)
            .saturating_sub(now_ms())
            .max(1) as u64
    };
    let timeout = Duration::from_millis(remaining.min(approval.policy.max_stage_seconds * 1000));
    let mut changes = app.inner.changes.subscribe();
    let transport = async {
        input
            .write_all(&bytes)
            .await
            .map_err(|_| error("driver_transport_failed"))?;
        input
            .shutdown()
            .await
            .map_err(|_| error("driver_transport_failed"))?;
        drop(input);
        let (status, out, stderr) = tokio::try_join!(
            async { child.wait().await.map_err(|_| ()) },
            read_bounded(output, max),
            read_bounded(stderr, max)
        )
        .map_err(|_| {
            error("driver_transport_failed").with_diagnostic(crate::diagnostics::Failure::new(
                crate::diagnostics::Stage::Driver,
                crate::diagnostics::Category::Transport,
            ))
        })?;
        if !status.success() {
            let failure = crate::diagnostics::Failure::parse(&stderr).unwrap_or_else(|| {
                crate::diagnostics::Failure::process(
                    crate::diagnostics::Stage::Driver,
                    status,
                    &[],
                    &[],
                )
            });
            return Err(error("driver_transport_failed").with_diagnostic(failure));
        }
        serde_json::from_slice::<Receipt>(&out).map_err(|_| error("driver_receipt_invalid"))
    };
    let cancelled = async {
        loop {
            let stop = app.inner.stopping.load(Ordering::SeqCst)
                || (!matches!(action, "cancel" | "observe")
                    && app
                        .inner
                        .store
                        .lock()
                        .ok()
                        .and_then(|s| s.current_run(field(run, "id").unwrap_or("")).ok())
                        .is_some_and(|r| r["status"] == "cancel_requested"));
            if stop {
                break;
            }
            let _ = changes.changed().await;
        }
    };
    let result = tokio::select! {value=tokio::time::timeout(timeout,transport)=>match value{Ok(Ok(receipt))=>Ok(receipt),Ok(Err(error))=>Err(error),Err(_)=>Err(error("deadline_exceeded"))},_ = cancelled=>Err(error("driver_interrupted"))};
    let group_clean = cleanup_group(&mut child, pid).await;
    if !group_clean {
        return Err(error("worker_cleanup_unproven"));
    }
    result.map(|receipt| DriverOutcome::Completed(Box::new(receipt)))
}
pub(crate) async fn cleanup_group(child: &mut tokio::process::Child, pid: u32) -> bool {
    #[cfg(unix)]
    {
        if unsafe { libc::kill(-(pid as i32), 0) } == -1
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            let _ = child.wait().await;
            return true;
        }
        // Never signal the host group: each approved driver starts in its own group.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGTERM);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
        let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
        for _ in 0..20 {
            if unsafe { libc::kill(-(pid as i32), 0) } == -1
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        false
    }
    #[cfg(not(unix))]
    {
        let _ = child.start_kill();
        let _ = child.wait().await;
        false
    }
}
async fn git(approval: &Approval, workspace: &str, args: &[&str]) -> Result<String> {
    approval.verify(&approval.digest, true)?;
    let mut cmd = Command::new(&approval.policy.git.executable);
    cmd.env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "protocol.file.allow=never",
            "-C",
            workspace,
        ])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().map_err(|_| error("git_verification_failed"))?;
    let pid = child.id().ok_or_else(|| error("git_verification_failed"))?;
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let (status, out, _) = tokio::try_join!(
            async { child.wait().await.map_err(|_| ()) },
            read_bounded(out, 16384),
            read_bounded(err, 16384)
        )?;
        if !status.success() {
            return Err(());
        }
        String::from_utf8(out).map_err(|_| ())
    })
    .await;
    if !cleanup_group(&mut child, pid).await {
        return Err(error("git_cleanup_unproven"));
    }
    result
        .map_err(|_| error("git_verification_failed"))?
        .map_err(|_| error("git_verification_failed"))
}

#[cfg(all(test, unix))]
mod control_tests {
    use super::*;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use std::os::unix::fs::PermissionsExt;
    #[tokio::test]
    async fn controls_winning_after_prepare_intent_but_before_spawn_preserve_zero_effect_proof() {
        for control in ["pause", "stop", "shutdown"] {
            let temporary = tempfile::tempdir().unwrap();
            let dir = temporary.path().canonicalize().unwrap();
            let work = dir.join("work");
            std::fs::create_dir(&work).unwrap();
            std::fs::set_permissions(&work, std::fs::Permissions::from_mode(0o700)).unwrap();
            let script = dir.join("driver.py");
            std::fs::write(&script, include_str!("../tests/fixtures/driver.py")).unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
            std::fs::write(dir.join("mode.json"), b"{}").unwrap();
            let program = json!({"executable":script,"sha256":crate::execution_config::digest_file(&script).unwrap(),"credential_env_names":[]});
            let git = json!({"executable":"/usr/bin/git","sha256":crate::execution_config::digest_file(Path::new("/usr/bin/git")).unwrap(),"credential_env_names":[]});
            let payload = json!({"schema_version":1,"containment_contract":"reviewed-launcher-v1","expires_at_ms":now_ms()+60000,"work_root":work,"worker":program,"publisher":program,"git":git,"max_run_seconds":60,"max_stage_seconds":10,"max_output_bytes":8192});
            let key = b"synthetic-control-test-key-0123456789";
            let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
            mac.update(payload.to_string().as_bytes());
            let config = dir.join("approval.json");
            std::fs::write(
                &config,
                json!({"payload":payload,"signature":format!("{:x}",mac.finalize().into_bytes())})
                    .to_string(),
            )
            .unwrap();
            std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
            let approval = Approval::load(
                &config,
                key,
                &crate::execution_config::digest_file(&config).unwrap(),
            )
            .unwrap();
            let app = App::open_approved(
                &dir.join("board.sqlite"),
                std::str::from_utf8(key).unwrap(),
                4311,
                Some(approval.clone()),
            )
            .unwrap();
            let (run, attempt) = {
                let mut store = app.inner.store.lock().unwrap();
                let task=store.command("create","create",None,"create",json!({"title":"Synthetic only","objective":"Control arbitration","repository":"fixture/repository","workflow":"engineering","status":"ready"})).unwrap().1["task"].clone();
                store
                    .execution_command(
                        "resume",
                        "resume",
                        "pause",
                        None,
                        json!({"paused":false}),
                        Some(&approval),
                    )
                    .unwrap();
                let admitted = store
                    .execution_command(
                        "run",
                        "run",
                        "run",
                        task["id"].as_str(),
                        json!({"expected_revision":task["revision"]}),
                        Some(&approval),
                    )
                    .unwrap()
                    .1["run"]
                    .clone();
                let (run, attempt) = store.next_attempt().unwrap().unwrap();
                assert_eq!(run["id"], admitted["id"]);
                assert!(run["last_effect_attempt"].is_string());
                if control == "pause" {
                    store
                        .execution_command(
                            "control",
                            "control",
                            "pause",
                            None,
                            json!({"paused":true}),
                            Some(&approval),
                        )
                        .unwrap();
                } else if control == "stop" {
                    store
                        .execution_command(
                            "control",
                            "control",
                            "cancel",
                            run["id"].as_str(),
                            json!({}),
                            Some(&approval),
                        )
                        .unwrap();
                } else {
                    app.inner.stopping.store(true, Ordering::SeqCst);
                }
                (run, attempt)
            };
            let (next, receipt) = perform(&app, &run, &attempt).await.unwrap();
            assert!(receipt.is_none());
            assert!(next["last_effect_attempt"].is_null());
            assert!(!dir.join("audit.jsonl").exists());
            assert!(!work.join(run["id"].as_str().unwrap()).exists());
            app.inner
                .store
                .lock()
                .unwrap()
                .finish_attempt(
                    run["id"].as_str().unwrap(),
                    attempt["id"].as_str().unwrap(),
                    next,
                    receipt,
                )
                .unwrap();
            let app = if control == "shutdown" {
                drop(app);
                App::open_approved(
                    &dir.join("board.sqlite"),
                    std::str::from_utf8(key).unwrap(),
                    4311,
                    Some(approval.clone()),
                )
                .unwrap()
            } else {
                app
            };
            let (cancel, cleanup) = {
                let mut store = app.inner.store.lock().unwrap();
                if control != "stop" {
                    store
                        .execution_command(
                            "stop",
                            "stop",
                            "cancel",
                            run["id"].as_str(),
                            json!({}),
                            Some(&approval),
                        )
                        .unwrap();
                }
                store.next_attempt().unwrap().unwrap()
            };
            assert_eq!(cleanup["action"], "cancel");
            let (closed, receipt) = perform(&app, &cancel, &cleanup).await.unwrap();
            assert_eq!(closed["status"], "cancelled");
            app.inner
                .store
                .lock()
                .unwrap()
                .finish_attempt(
                    cancel["id"].as_str().unwrap(),
                    cleanup["id"].as_str().unwrap(),
                    closed,
                    receipt,
                )
                .unwrap();
            assert!(
                app.inner.store.lock().unwrap().execution_state().unwrap()["lane_run_id"].is_null()
            );
            let audit = std::fs::read_to_string(dir.join("audit.jsonl")).unwrap();
            assert_eq!(audit.lines().count(), 1);
            assert!(audit.contains("\"action\": \"cancel\""));
        }
    }
    #[test]
    fn missing_legacy_run_qa_policy_never_uses_skip_receipt() {
        let mut run = json!({"task_snapshot":{},"qa_receipt":null,"coder_receipt":{"passed":true}});
        assert!(verify_qa(&run).is_err());
        run["qa_receipt"] = json!({"passed":true,"role":"qa","reviewed_sha":"head","session_id":"independent","cleanup_complete":true});
        run["head_sha"] = json!("head");
        run["coder_session"] = json!("coder");
        assert!(verify_qa(&run).is_ok());
    }
}
