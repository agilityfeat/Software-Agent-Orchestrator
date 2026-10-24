use crate::{
    ApiError, Result,
    execution_config::{Approval, now_ms},
    store::{Store, get, internal, now, persist},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use uuid::Uuid;

pub fn terminal(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "cancelled")
}
fn run(db: &Connection, id: &str) -> Result<Value> {
    let row: Option<String> = db
        .query_row("SELECT document FROM runs WHERE id=?1", [id], |r| r.get(0))
        .optional()
        .map_err(internal)?;
    serde_json::from_str(&row.ok_or_else(|| ApiError::new(404, "run_not_found"))?).map_err(internal)
}
pub(crate) fn save(db: &Connection, value: &Value) -> Result<()> {
    db.execute("INSERT INTO runs(id,task_id,document) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET document=excluded.document",params![value["id"].as_str(),value["task_id"].as_str(),value.to_string()]).map_err(internal)?;
    Ok(())
}
pub(crate) fn event(db: &Connection, value: &Value, kind: &str) -> Result<()> {
    let doc = json!({"type":kind,"run_id":value["id"],"task_id":value["task_id"],"status":value["status"],"stage":value["stage"],"reason":value["reason"],"at":now()});
    db.execute(
        "INSERT INTO events(task_id,document) VALUES(?1,?2)",
        params![value["task_id"].as_str(), doc.to_string()],
    )
    .map_err(internal)?;
    Ok(())
}
pub fn public_run(mut value: Value) -> Value {
    value["evidence"] = json!({"coder_status":if value["coder_session"].is_string(){"complete"}else{"unavailable"},"qa_status":if value["task_snapshot"]["qa_policy"]=="skip" {"skipped"}else if value["qa_receipt"]["passed"]==true {"passed"}else if value["qa_receipt"]["passed"]==false {"failed"}else{"unavailable"},"review_status":"not_required"});
    if let Some(obj) = value.as_object_mut() {
        obj.remove("task_snapshot");
        obj.remove("workspace");
        obj.remove("qa_receipt");
        obj.remove("coder_receipt");
        obj.remove("runtime_approval_path");
        obj.remove("session_history");
        obj.remove("repair_context");
        if let Some(history) = obj.get_mut("review_history").and_then(Value::as_array_mut) {
            for cycle in history {
                if let Some(o) = cycle.as_object_mut() {
                    o.remove("coder_session_id");
                    o.remove("qa_session_id");
                }
            }
        }
    }
    value
}
#[derive(Default)]
pub(crate) struct Admission<'a> {
    pub approval: Option<&'a Approval>,
    pub ownership: Option<&'a std::sync::Mutex<crate::ownership::Guard>>,
    pub automatic: Option<&'a crate::automation::Activation>,
    pub repository_queue: bool,
    pub coding_agents: Option<&'a crate::adapters::native::NativeAgentPolicy>,
    pub agent_registry: Option<&'a crate::system::Registry>,
}
impl Store {
    pub fn runs(&self, task_id: Option<&str>) -> Result<Vec<Value>> {
        let mut q = self
            .db
            .prepare("SELECT document FROM runs WHERE (?1 IS NULL OR task_id=?1) ORDER BY rowid")
            .map_err(internal)?;
        let rows = q
            .query_map([task_id], |r| r.get::<_, String>(0))
            .map_err(internal)?;
        let mut values = Vec::new();
        for row in rows {
            values.push(public_run(
                serde_json::from_str(&row.map_err(internal)?).map_err(internal)?,
            ));
        }
        Ok(values)
    }
    pub fn execution_state(&self) -> Result<Value> {
        self.db.query_row("SELECT paused,lane_run_id FROM execution WHERE singleton=1",[],|r|Ok(json!({"paused":r.get::<_,bool>(0)?,"lane_run_id":r.get::<_,Option<String>>(1)?}))).map_err(internal)
    }
    pub fn recover_runs(&mut self) -> Result<()> {
        let tx = self.db.transaction().map_err(internal)?;
        let pending = {
            let mut q=tx.prepare("SELECT document FROM runs WHERE json_extract(document,'$.status') NOT IN ('succeeded','failed','cancelled')").map_err(internal)?;
            q.query_map([], |r| r.get::<_, String>(0))
                .map_err(internal)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(internal)?
        };
        for raw in pending {
            let mut value: Value = serde_json::from_str(&raw).map_err(internal)?;
            value["status"] = json!("unknown");
            value["reason"] = json!("restart_requires_reconciliation");
            value["reconcile_requested"] = json!(false);
            value["updated_at"] = json!(now());
            if let Some(id) = value["project_id"].as_str() {
                let mut p = crate::projects::get(&tx, id)?;
                p["paused"] = json!(true);
                p["failure"] = json!("restart_requires_reconciliation");
                p["revision"] = json!(p["revision"].as_u64().unwrap_or(0) + 1);
                p["updated_at"] = json!(now());
                crate::projects::save(&tx, &p)?;
            }
            save(&tx, &value)?;
            event(&tx, &value, "run.updated")?;
        }
        tx.execute("UPDATE attempts SET document=json_set(document,'$.state','unknown') WHERE json_extract(document,'$.state')='attempted'",[]).map_err(internal)?;
        tx.commit().map_err(internal)?;
        Ok(())
    }
    #[cfg(test)]
    pub fn execution_command(
        &mut self,
        key: &str,
        fingerprint: &str,
        scope: &str,
        id: Option<&str>,
        body: Value,
        approval: Option<&Approval>,
    ) -> Result<(u16, Value)> {
        self.execution_command_guarded(
            key,
            fingerprint,
            scope,
            id,
            body,
            Admission {
                approval,
                ..Admission::default()
            },
        )
    }
    pub fn execution_command_guarded(
        &mut self,
        key: &str,
        fingerprint: &str,
        scope: &str,
        id: Option<&str>,
        body: Value,
        admission: Admission<'_>,
    ) -> Result<(u16, Value)> {
        let Admission {
            approval,
            ownership,
            automatic,
            ..
        } = admission;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(internal)?;
        if let Some((prior, status, response)) = tx
            .query_row(
                "SELECT fingerprint,status,response FROM commands WHERE key=?1",
                [key],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, u16>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(internal)?
        {
            if prior != fingerprint {
                return Err(ApiError::conflict("idempotency_conflict"));
            }
            return Ok((status, serde_json::from_str(&response).map_err(internal)?));
        }
        let fields = body
            .as_object()
            .ok_or_else(|| ApiError::bad("request_invalid"))?;
        let (status, response) = match scope {
            "pause" => {
                if fields.len() != 1 || !body["paused"].is_boolean() {
                    return Err(ApiError::bad("request_invalid"));
                }
                tx.execute(
                    "UPDATE execution SET paused=?1 WHERE singleton=1",
                    [body["paused"].as_bool().unwrap()],
                )
                .map_err(internal)?;
                let document =
                    json!({"type":"execution.updated","paused":body["paused"],"at":now()});
                tx.execute(
                    "INSERT INTO events(task_id,document) VALUES(NULL,?1)",
                    [document.to_string()],
                )
                .map_err(internal)?;
                (200, json!({"execution":{"paused":body["paused"]}}))
            }
            "run" => {
                let research_task = get(&tx, id.unwrap_or(""))?;
                if crate::research::eligible(&research_task) {
                    if let Some(registry) = admission.agent_registry {
                        registry.assignment(Some(&research_task))?;
                    }
                    crate::research::admit(&tx, &research_task, &body, approval)?
                } else {
                    if let Some(registry) = admission.agent_registry {
                        registry.assignment(Some(&get(&tx, id.unwrap_or(""))?))?;
                    }
                    let approval =
                        approval.ok_or_else(|| ApiError::unavailable("execution_unavailable"))?;
                    if let Some(config) = approval
                        .policy
                        .adapter_policy
                        .as_ref()
                        .and_then(|p| p.agent_config.as_ref())
                    {
                        if crate::system::get(&tx)?.revision != config.revision {
                            return Err(ApiError::conflict("agent_config_changed"));
                        }
                    }
                    approval.verify(&approval.digest, false)?;
                    if fields.len() != 1 || body["expected_revision"].as_u64().is_none() {
                        return Err(ApiError::conflict("revision_required"));
                    }
                    let mut task = get(&tx, id.unwrap_or(""))?;
                    if let Some(native) = approval
                        .policy
                        .adapter_policy
                        .as_ref()
                        .and_then(|p| p.native_agent.as_ref())
                    {
                        task["coding_agent_id"] =
                            json!(native.agent(task["coding_agent_id"].as_str(), false)?.id);
                    }
                    if approval.policy.adapter_policy.as_ref().is_some_and(|p| {
                        !task["repository"]
                            .as_str()
                            .is_some_and(|r| p.repository.eq_ignore_ascii_case(r))
                    }) {
                        return Err(ApiError::conflict("pilot_repository_mismatch"));
                    }
                    if let Some(guard) = ownership {
                        guard
                            .lock()
                            .map_err(|_| ApiError::internal())?
                            .assert(task["repository"].as_str().unwrap_or(""))?;
                    }
                    if task["revision"] != body["expected_revision"] {
                        return Err(ApiError::conflict("revision_conflict"));
                    }
                    if task["workflow"] != "engineering"
                        || !["ready"].contains(&task["status"].as_str().unwrap_or(""))
                        || !task["blocked_reason"].is_null()
                        || !task["archived_at"].is_null()
                    {
                        return Err(ApiError::conflict("task_not_runnable"));
                    }
                    crate::tickets::validate_size(&task)?;
                    if !crate::tickets::blockers(&tx, &task)?.is_empty() {
                        return Err(ApiError::conflict("dependencies_unmet"));
                    }
                    let (paused, lane): (bool, Option<String>) = tx
                        .query_row(
                            "SELECT paused,lane_run_id FROM execution WHERE singleton=1",
                            [],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .map_err(internal)?;
                    if paused {
                        return Err(ApiError::conflict("execution_paused"));
                    }
                    if lane.is_some() {
                        return Err(ApiError::conflict("active_run_exists"));
                    }
                    let project = if let Some(id) = task["project_id"].as_str() {
                        Some(crate::projects::get(&tx, id)?)
                    } else {
                        crate::projects::by_repository(
                            &tx,
                            task["repository"].as_str().unwrap_or(""),
                        )?
                    };
                    if let Some(p) = &project {
                        task["repository"] = p["repository"].clone();
                        task["project_repository_id"] = p["repository_id"].clone();
                        task["project_default_branch"] = p["default_branch"].clone();
                    }
                    if let Some(p) = &project {
                        if p["paused"] != false {
                            return Err(ApiError::conflict("project_paused"));
                        }
                        if !p["lane_run_id"].is_null() {
                            return Err(ApiError::conflict("active_run_exists"));
                        }
                        if task["project_id"].is_null()
                            && (p["runtime_status"] != "configured"
                                || p["checked_task_revision"]
                                    != format!(
                                        "{}:{}",
                                        task["id"].as_str().unwrap_or(""),
                                        task["revision"]
                                    ))
                        {
                            return Err(ApiError::conflict("project_needs_preparation"));
                        }
                        let prior:i64=tx.query_row("SELECT count(*) FROM runs WHERE task_id=?1 AND json_extract(document,'$.task_revision')=?2",params![task["id"].as_str(),task["revision"].as_u64()],|r|r.get(0)).map_err(internal)?;
                        let used:i64=tx.query_row("SELECT count(*) FROM runs WHERE json_extract(document,'$.project_id')=?1",[p["id"].as_str()],|r|r.get(0)).map_err(internal)?;
                        if prior > 0 {
                            return Err(ApiError::conflict("automatic_run_already_consumed"));
                        }
                        if used >= p["max_runs"].as_i64().unwrap_or(0) {
                            return Err(ApiError::conflict("project_budget_exhausted"));
                        }
                        let expected = p["approval_sha256"].as_str().unwrap_or("");
                        if let Some(config) = approval
                            .policy
                            .adapter_policy
                            .as_ref()
                            .and_then(|p| p.agent_config.as_ref())
                        {
                            if config.base_approval_sha256.as_deref() != Some(expected) {
                                return Err(ApiError::conflict("execution_approval_changed"));
                            }
                            approval.verify(&approval.digest, false)?;
                        } else {
                            approval.verify(expected, false)?;
                        }
                    }
                    let repository_busy:i64=tx.query_row("SELECT count(*) FROM runs WHERE lower(json_extract(document,'$.repository'))=lower(?1) AND json_extract(document,'$.status') NOT IN ('succeeded','failed','cancelled')",[task["repository"].as_str()],|r|r.get(0)).map_err(internal)?;
                    if repository_busy > 0 {
                        return Err(ApiError::conflict("active_run_exists"));
                    }
                    if project.is_none() {
                        let active: i64 = tx
                            .query_row("SELECT count(*) FROM project_lanes", [], |r| r.get(0))
                            .map_err(internal)?;
                        if active > 0 {
                            return Err(ApiError::conflict("active_run_exists"));
                        }
                    }
                    if let Some(activation) = automatic {
                        activation.verify(approval)?;
                        if !activation.eligible(&task) {
                            return Err(ApiError::conflict("automatic_run_not_eligible"));
                        }
                        let consumed:i64=tx.query_row("SELECT count(*) FROM automation_consumptions WHERE activation_sha256=?1",[&activation.digest],|r|r.get(0)).map_err(internal)?;
                        let prior:i64=tx.query_row("SELECT count(*) FROM runs WHERE task_id=?1 AND json_extract(document,'$.task_revision')=?2",params![task["id"].as_str(),task["revision"].as_u64()],|r|r.get(0)).map_err(internal)?;
                        if consumed > 0 || prior > 0 {
                            return Err(ApiError::conflict("automatic_run_already_consumed"));
                        }
                    }
                    let run_id = Uuid::new_v4().to_string();
                    let workspace = approval.policy.work_root.join(&run_id);
                    let value = json!({"id":run_id,"task_id":task["id"],"task_revision":task["revision"],"repository":task["repository"],"task_snapshot":task,"approval_sha256":approval.digest,"workspace":workspace,"status":"admitted","stage":"prepare","base_sha":null,"head_sha":null,"coder_session":null,"qa_receipt":null,"pull_request_url":null,"merge_commit_sha":null,"reason":null,"last_attempt":null,"receipt_count":0,"reconcile_requested":false,"created_at":now(),"updated_at":now(),"deadline_ms":now_ms()+approval.policy.max_run_seconds as i64*1000});
                    let mut value = value;
                    if let Some(config) = approval
                        .policy
                        .adapter_policy
                        .as_ref()
                        .and_then(|p| p.agent_config.as_ref())
                    {
                        value["agents"] = config.public();
                    }
                    value["max_qa_repairs"] = json!(approval.policy.max_qa_repairs.unwrap_or(2));
                    value["repair_count"] = json!(0);
                    value["review_history"] = json!([]);
                    value["session_history"] = json!([]);
                    if let Some(activation) = automatic {
                        value["automation"] = json!({"activation_sha256":activation.digest,"revision":task["revision"]});
                    }
                    if let Some(p) = &project {
                        value["project_id"] = p["id"].clone();
                        value["project_revision"] = p["revision"].clone();
                        value["runtime_approval_path"] = json!(approval.path());
                        value["automation"] =
                            json!({"revision":task["revision"],"project_id":p["id"]});
                    }
                    save(&tx, &value)?;
                    if let Some(activation) = automatic {
                        tx.execute(
                            "INSERT INTO automation_consumptions VALUES(?1,?2)",
                            params![activation.digest, run_id],
                        )
                        .map_err(internal)?;
                    }
                    if let Some(mut p) = project {
                        tx.execute(
                            "INSERT INTO project_lanes VALUES(?1,?2,?3)",
                            params![
                                p["id"].as_str(),
                                run_id,
                                task["repository"].as_str().unwrap_or("").to_lowercase()
                            ],
                        )
                        .map_err(internal)?;
                        p["lane_run_id"] = json!(run_id);
                        p["revision"] = json!(p["revision"].as_u64().unwrap_or(0) + 1);
                        p["updated_at"] = json!(now());
                        crate::projects::save(&tx, &p)?;
                    } else {
                        tx.execute(
                            "UPDATE execution SET lane_run_id=?1 WHERE singleton=1",
                            [&run_id],
                        )
                        .map_err(internal)?;
                    }
                    task["status"] = json!("active");
                    task["revision"] = json!(task["revision"].as_u64().unwrap() + 1);
                    task["updated_at"] = json!(now());
                    persist(&tx, &task, "task.updated")?;
                    event(&tx, &value, "run.created")?;
                    (202, json!({"run":public_run(value)}))
                }
            }
            "cancel" | "reconcile" => {
                if !fields.is_empty() {
                    return Err(ApiError::bad("request_invalid"));
                }
                let mut value = run(&tx, id.unwrap_or(""))?;
                if terminal(value["status"].as_str().unwrap_or("")) {
                    return Err(ApiError::conflict("run_closed"));
                }
                let approved =
                    approval.ok_or_else(|| ApiError::unavailable("execution_unavailable"))?;
                approved.verify(value["approval_sha256"].as_str().unwrap_or(""), true)?;
                if scope == "reconcile" {
                    if !["unknown", "awaiting_merge"]
                        .contains(&value["status"].as_str().unwrap_or(""))
                    {
                        return Err(ApiError::conflict("run_not_reconcilable"));
                    }
                    value["reconcile_requested"] = json!(true);
                } else {
                    value["status"] = json!("cancel_requested");
                }
                value["updated_at"] = json!(now());
                save(&tx, &value)?;
                event(&tx, &value, "run.updated")?;
                (202, json!({"run":public_run(value)}))
            }
            _ => return Err(ApiError::bad("request_invalid")),
        };
        tx.execute(
            "INSERT INTO commands VALUES(?1,?2,?3,?4)",
            params![key, fingerprint, status, response.to_string()],
        )
        .map_err(internal)?;
        tx.commit().map_err(internal)?;
        Ok((status, response))
    }
    /// Persists intent before returning a spawnable action. Replays never create another attempt.
    pub fn next_attempt(&mut self) -> Result<Option<(Value, Value)>> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(internal)?;
        let paused: bool = tx
            .query_row("SELECT paused FROM execution WHERE singleton=1", [], |r| {
                r.get(0)
            })
            .map_err(internal)?;
        let candidates = {
            let mut q=tx.prepare("SELECT document FROM runs WHERE id IN (SELECT run_id FROM project_lanes UNION SELECT lane_run_id FROM execution WHERE lane_run_id IS NOT NULL UNION SELECT id FROM runs WHERE json_extract(document,'$.kind')='research' AND json_extract(document,'$.status') NOT IN ('succeeded','failed','cancelled')) ORDER BY rowid").map_err(internal)?;
            q.query_map([], |r| r.get::<_, String>(0))
                .map_err(internal)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(internal)?
        };
        let mut selected = None;
        for raw in candidates {
            let value: Value = serde_json::from_str(&raw).map_err(internal)?;
            let project_paused = value["project_id"]
                .as_str()
                .map(|id| crate::projects::get(&tx, id).map(|p| p["paused"] != false))
                .transpose()?
                .unwrap_or(false);
            let status = value["status"].as_str().unwrap_or("");
            let action = if status == "cancel_requested" {
                "cancel"
            } else if value["reconcile_requested"] == true
                || (status == "running" && value["stage"] == "observe_cancel")
            {
                "observe"
            } else if matches!(status, "admitted" | "running") && !paused && !project_paused {
                value["stage"].as_str().unwrap_or("")
            } else {
                continue;
            };
            if action.is_empty() {
                continue;
            }
            if let Some(last) = value["last_attempt"].as_str() {
                let raw: String = tx
                    .query_row("SELECT document FROM attempts WHERE id=?1", [last], |r| {
                        r.get(0)
                    })
                    .map_err(internal)?;
                let prior: Value = serde_json::from_str(&raw).map_err(internal)?;
                if prior["state"] == "attempted" {
                    continue;
                }
            }
            selected = Some((value.clone(), action.to_string()));
            break;
        }
        let Some((mut value, action)) = selected else {
            return Ok(None);
        };
        let id = value["id"].as_str().unwrap().to_string();
        let action = action.as_str();
        let driver =
            if crate::research::is_run(&value) || matches!(action, "coder" | "qa" | "cancel") {
                "worker"
            } else {
                "publisher"
            };
        let attempt = json!({"id":Uuid::new_v4().to_string(),"run_id":id,"action":action,"driver":driver,"state":"attempted","observed_action_id":value["last_attempt"],"at":now()});
        tx.execute(
            "INSERT INTO attempts VALUES(?1,?2,?3)",
            params![attempt["id"].as_str(), id, attempt.to_string()],
        )
        .map_err(internal)?;
        if !matches!(action, "observe" | "cancel") {
            value["last_effect_attempt"] = attempt["id"].clone();
        }
        value["last_attempt"] = attempt["id"].clone();
        value["reconcile_requested"] = json!(false);
        if !matches!(action, "observe" | "cancel") {
            value["status"] = json!("running");
        }
        value["updated_at"] = json!(now());
        save(&tx, &value)?;
        event(&tx, &value, "run.updated")?;
        tx.commit().map_err(internal)?;
        Ok(Some((value, attempt)))
    }
    pub fn current_run(&self, id: &str) -> Result<Value> {
        run(&self.db, id)
    }
    pub fn finish_attempt(
        &mut self,
        id: &str,
        attempt_id: &str,
        mut value: Value,
        receipt: Option<Value>,
    ) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(internal)?;
        let current = run(&tx, id)?;
        // A concurrent stop wins over stage continuation; exact delivered success remains factual.
        let mut attempt: Value = serde_json::from_str(
            &tx.query_row::<String, _, _>(
                "SELECT document FROM attempts WHERE id=?1",
                [attempt_id],
                |r| r.get(0),
            )
            .map_err(internal)?,
        )
        .map_err(internal)?;
        if crate::research::is_run(&current)
            && current["status"] == "cancel_requested"
            && attempt["action"] != "cancel"
            && value["status"] != "cancelled"
        {
            // A report has no external publication to preserve against acknowledged Stop.
            value["status"] = json!("cancel_requested");
        }
        if current["status"] == "cancel_requested"
            && attempt["action"] != "cancel"
            && value["status"] != "succeeded"
            && value["status"] != "cancelled"
            && value["stage"] != "observe_cancel"
        {
            value["status"] = json!("cancel_requested");
        }
        attempt["state"] = json!(if value["status"] == "unknown" {
            "unknown"
        } else {
            "observed"
        });
        attempt["receipt"] = receipt.unwrap_or(Value::Null);
        attempt["finished_at"] = json!(now());
        tx.execute(
            "UPDATE attempts SET document=?1 WHERE id=?2",
            params![attempt.to_string(), attempt_id],
        )
        .map_err(internal)?;
        value["updated_at"] = json!(now());
        value["receipt_count"] = json!(
            value["receipt_count"].as_u64().unwrap_or(0) + u64::from(!attempt["receipt"].is_null())
        );
        // Only a newly validated QA cycle is eligible. Unknown/error outcomes do
        // not append this entry; ticket links and receipt observation commit together.
        if attempt["action"] == "qa" && value["status"] != "unknown" {
            if let Some(cycle) = value["review_history"]
                .as_array_mut()
                .and_then(|h| h.iter_mut().find(|c| c["qa_action_id"] == attempt_id))
            {
                let bugs: Vec<crate::qa_feedback::OutOfScopeBug> =
                    serde_json::from_value(cycle["out_of_scope_bugs"].clone()).map_err(internal)?;
                crate::qa_feedback::out_of_scope_bugs(
                    &bugs,
                    current["base_sha"].as_str().unwrap_or(""),
                )?;
                let mut links = Vec::new();
                for bug in bugs {
                    links.push(bug_ticket(
                        &tx,
                        &current,
                        &bug,
                        cycle["reviewed_sha"].as_str().unwrap_or(""),
                        attempt_id,
                    )?);
                }
                cycle["bug_task_ids"] = json!(links);
            }
        }
        if let Some(project_id) = value["project_id"].as_str() {
            let mut p = crate::projects::get(&tx, project_id)?;
            if matches!(value["status"].as_str(), Some("failed" | "unknown")) {
                p["paused"] = json!(true);
                p["failure"] = value["reason"].clone();
            }
            if terminal(value["status"].as_str().unwrap_or("")) {
                tx.execute("DELETE FROM project_lanes WHERE run_id=?1", [id])
                    .map_err(internal)?;
                p["lane_run_id"] = Value::Null;
            }
            if terminal(value["status"].as_str().unwrap_or("")) || value["status"] == "unknown" {
                p["revision"] = json!(p["revision"].as_u64().unwrap_or(0) + 1);
            }
            p["updated_at"] = json!(now());
            crate::projects::save(&tx, &p)?;
        }
        if terminal(value["status"].as_str().unwrap_or("")) {
            tx.execute(
                "UPDATE execution SET lane_run_id=NULL WHERE lane_run_id=?1",
                [id],
            )
            .map_err(internal)?;
            let mut task = get(&tx, value["task_id"].as_str().unwrap())?;
            task["status"] = json!(if value["status"] == "succeeded" {
                "done"
            } else {
                "ready"
            });
            task["ready_at"] = if value["status"] == "succeeded" {
                Value::Null
            } else {
                json!(now())
            };
            task["revision"] = json!(task["revision"].as_u64().unwrap() + 1);
            task["updated_at"] = json!(now());
            if value["status"] == "succeeded" {
                task["completed_at"] = json!(now());
            }
            persist(&tx, &task, "task.updated")?;
        }
        save(&tx, &value)?;
        event(&tx, &value, "run.updated")?;
        tx.commit().map_err(internal)?;
        Ok(())
    }
}

fn bug_ticket(
    tx: &rusqlite::Transaction<'_>,
    run: &Value,
    bug: &crate::qa_feedback::OutOfScopeBug,
    reviewed_sha: &str,
    action_id: &str,
) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut identity = serde_json::to_value(bug).map_err(internal)?;
    identity.as_object_mut().unwrap().remove("base_sha");
    identity.as_object_mut().unwrap().remove("criticality");
    let fingerprint = format!("{:x}",Sha256::digest(json!({"repository":run["repository"].as_str().unwrap_or("").to_ascii_lowercase(),"bug":identity}).to_string().as_bytes()));
    if let Some(id) = tx
        .query_row(
            "SELECT id FROM tasks WHERE json_extract(document,'$.qa_bug_fingerprint')=?1",
            [&fingerprint],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(internal)?
    {
        if !get(tx, &id)?["repository"]
            .as_str()
            .is_some_and(|repo| repo.eq_ignore_ascii_case(run["repository"].as_str().unwrap_or("")))
        {
            return Err(ApiError::conflict("qa_bug_repository_mismatch"));
        }
        return Ok(id);
    }
    let count: i64 = tx
        .query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))
        .map_err(internal)?;
    if count >= 10000 {
        return Err(ApiError::conflict("task_limit_reached"));
    }
    let id = Uuid::new_v4().to_string();
    let at = now();
    let source = &run["task_snapshot"];
    let task = json!({"id":id,"revision":1,"title":bug.summary,"objective":format!("Fix the independently reported pre-existing bug at {}:{}.\nReproduction: {}\nExpected: {}\nActual: {}\nBase reproduction ({}): {}\nWhy unrelated to the source task: {}",bug.path,bug.line,bug.reproduction,bug.expected,bug.actual,bug.base_sha,bug.base_reproduction,bug.unrelated_reason),"acceptance_criteria":[bug.expected,"Add a regression test demonstrating the reported reproduction.","Independent QA verifies the fix and related regressions."],"context":[],"project":source["project"],"project_id":run["project_id"],"repository":run["repository"],"owner":"Coder","priority":bug.criticality.priority(),"criticality":bug.criticality,"status":"backlog","workflow":"engineering","task_type":"bug","labels":[],"dependencies":[],"qa_policy":"required","merge_policy":"auto","automation_policy":"manual","automation_revision":null,"ready_at":null,"blocked_reason":null,"github_issue_url":null,"github_pull_request_url":null,"archived_at":null,"completed_at":null,"cancelled_at":null,"created_at":at,"updated_at":at,"qa_bug_fingerprint":fingerprint,"qa_source":{"task_id":run["task_id"],"run_id":run["id"],"reviewed_sha":reviewed_sha,"qa_action_id":action_id}});
    persist(tx, &task, "task.created")?;
    Ok(id)
}
