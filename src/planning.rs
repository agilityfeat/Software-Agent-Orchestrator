//! Explicit goal-based AI planning. Proposals confer no execution or task authority.
use crate::{
    ApiError, App, Result,
    execution::{Receipt, cleanup_group, read_bounded},
    execution_config::{Approval, now_ms},
    store::{Store, internal, now, persist},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{process::Stdio, sync::atomic::Ordering, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command};
use uuid::Uuid;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub title: String,
    pub objective: String,
    pub acceptance_criteria: Vec<String>,
    pub priority: u8,
    #[serde(default = "crate::tickets::feature")]
    pub task_type: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<usize>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub summary: String,
    pub tasks: Vec<Proposal>,
}
fn text(s: &str, min: usize, max: usize) -> bool {
    s.trim().chars().count() >= min
        && s.chars().count() <= max
        && !s
            .chars()
            .any(|c| c.is_control() && !['\n', '\t', '\r'].contains(&c))
}
impl Draft {
    pub fn validate(&self) -> Result<()> {
        if !text(&self.summary, 1, 2000)
            || self.tasks.is_empty()
            || self.tasks.len() > 8
            || serde_json::to_vec(self).map_err(internal)?.len() > 24576
        {
            return Err(ApiError::bad("planning_result_invalid"));
        }
        for (i, t) in self.tasks.iter().enumerate() {
            validate_proposal(t)?;
            let mut pending = t.depends_on.clone();
            let mut seen = std::collections::HashSet::new();
            while let Some(index) = pending.pop() {
                if index == i || index >= self.tasks.len() {
                    return Err(ApiError::bad("planning_result_invalid"));
                }
                if seen.insert(index) {
                    pending.extend(self.tasks[index].depends_on.iter().copied());
                }
            }
        }
        Ok(())
    }
}
fn validate_proposal(t: &Proposal) -> Result<()> {
    if !text(&t.title, 1, 240)
        || !text(&t.objective, 1, 4000)
        || t.acceptance_criteria.is_empty()
        || t.acceptance_criteria.len() > 8
        || t.acceptance_criteria.iter().any(|s| !text(s, 1, 1000))
        || !crate::tickets::TYPES.contains(&t.task_type.as_str())
        || crate::tickets::labels(&t.labels).is_err()
        || t.depends_on.len() > 8
        || t.depends_on
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != t.depends_on.len()
        || !(1..=4).contains(&t.priority)
    {
        return Err(ApiError::bad("planning_result_invalid"));
    }
    Ok(())
}
pub(crate) fn schema() -> Value {
    json!({"type":"object","additionalProperties":false,"required":["summary","tasks"],"properties":{"summary":{"type":"string"},"tasks":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["title","objective","acceptance_criteria","priority","task_type","labels","depends_on"],"properties":{"title":{"type":"string"},"objective":{"type":"string"},"acceptance_criteria":{"type":"array","items":{"type":"string"}},"priority":{"type":"integer","enum":[1,2,3,4]},"task_type":{"type":"string","enum":["feature","bug","improvement","chore","research"]},"labels":{"type":"array","items":{"type":"string"}},"depends_on":{"type":"array","items":{"type":"integer","minimum":0,"maximum":7}}}}}}})
}
pub(crate) fn public(mut v: Value) -> Value {
    if let Some(o) = v.as_object_mut() {
        o.remove("approval_path");
        o.remove("approval_sha256");
    }
    v
}
pub(crate) fn get(db: &Connection, id: &str) -> Result<Value> {
    let s: Option<String> = db
        .query_row("SELECT document FROM plans WHERE id=?1", [id], |r| r.get(0))
        .optional()
        .map_err(internal)?;
    serde_json::from_str(&s.ok_or_else(|| ApiError::new(404, "plan_not_found"))?).map_err(internal)
}
pub(crate) fn list(db: &Connection) -> Result<Vec<Value>> {
    let mut q = db
        .prepare("SELECT document FROM plans ORDER BY rowid DESC LIMIT 100")
        .map_err(internal)?;
    let rows = q
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(internal)?;
    rows.map(|r| {
        Ok(public(
            serde_json::from_str(&r.map_err(internal)?).map_err(internal)?,
        ))
    })
    .collect()
}
fn save(db: &Connection, v: &Value) -> Result<()> {
    db.execute(
        "INSERT INTO plans VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET document=excluded.document",
        params![v["id"].as_str(), v.to_string()],
    )
    .map_err(internal)?;
    let event = json!({"type":"plan.updated","plan_id":v["id"],"status":v["status"],"at":now(),"summary":"Main plan updated"});
    db.execute(
        "INSERT INTO events(task_id,document) VALUES(NULL,?1)",
        [event.to_string()],
    )
    .map_err(internal)?;
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Goal {
    goal: String,
    #[serde(default)]
    context: String,
    repository: String,
    project: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    proposal_id: String,
    proposal: Proposal,
    #[serde(default)]
    qa_policy: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Acceptance {
    expected_revision: u64,
    selected: Vec<Selection>,
}
impl Store {
    pub(crate) fn plan_command(
        &mut self,
        factory: Option<&crate::projects::Factory>,
        key: &str,
        fingerprint: &str,
        id: Option<&str>,
        action: &str,
        body: Value,
    ) -> Result<(u16, Value)> {
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
        let (status, response) = if action == "generate" {
            if serde_json::to_vec(&body).map_err(internal)?.len() > 24576 {
                return Err(ApiError::bad("planning_input_invalid"));
            }
            let input: Goal =
                serde_json::from_value(body).map_err(|_| ApiError::bad("request_invalid"))?;
            if !text(&input.goal, 1, 8000)
                || !text(&input.context, 0, 8000)
                || !text(&input.project, 1, 160)
                || !crate::store::valid_repository(&input.repository)
            {
                return Err(ApiError::bad("planning_input_invalid"));
            }
            let paused: bool = tx
                .query_row("SELECT paused FROM execution WHERE singleton=1", [], |r| {
                    r.get(0)
                })
                .map_err(internal)?;
            if paused {
                return Err(ApiError::conflict("execution_paused"));
            }
            let active:i64=tx.query_row("SELECT count(*) FROM plans WHERE json_extract(document,'$.status') IN ('queued','generating','unknown','cleanup_pending')",[],|r|r.get(0)).map_err(internal)?;
            if active > 0 {
                return Err(ApiError::conflict("planning_busy"));
            }
            let count: i64 = tx
                .query_row("SELECT count(*) FROM plans", [], |r| r.get(0))
                .map_err(internal)?;
            if count >= 1000 {
                return Err(ApiError::conflict("planning_limit_reached"));
            }
            let id = Uuid::new_v4().to_string();
            let f = factory.ok_or_else(|| ApiError::unavailable("planning_unavailable"))?;
            let registry = crate::system::get(&tx)?;
            let assignment = registry.assignment(None)?;
            let approval = f.plan_approval(&id, &input.repository)?;
            let policy = f.approval(&approval)?.with_agents(assignment.clone())?;
            let v = json!({"id":id,"revision":1,"status":"queued","goal":input.goal,"context":input.context,"repository":input.repository,"project":input.project,"source_inspected":false,"approval_path":policy.path(),"approval_sha256":policy.digest,"agent":assignment.public()["main"],"agent_revision":assignment.revision,"deadline_ms":now_ms()+policy.policy.max_stage_seconds.min(policy.policy.max_run_seconds) as i64*1000,"created_at":now(),"updated_at":now(),"proposals":[],"accepted_task_ids":[]});
            save(&tx, &v)?;
            (202, json!({"plan":public(v)}))
        } else {
            let mut v = get(&tx, id.ok_or_else(|| ApiError::bad("request_invalid"))?)?;
            if action == "accept" {
                let input: Acceptance =
                    serde_json::from_value(body).map_err(|_| ApiError::bad("request_invalid"))?;
                if v["status"] != "proposed" {
                    return Err(ApiError::conflict("plan_not_reviewable"));
                }
                if v["revision"].as_u64() != Some(input.expected_revision) {
                    return Err(ApiError::conflict("revision_conflict"));
                }
                if input.selected.is_empty() || input.selected.len() > 8 {
                    return Err(ApiError::bad("planning_selection_invalid"));
                }
                // Resolve proposal-local zero-based references against the persisted full plan.
                let originals = v["proposals"].as_array().ok_or_else(ApiError::internal)?;
                let selected_positions: Vec<usize> = input
                    .selected
                    .iter()
                    .map(|s| {
                        originals
                            .iter()
                            .position(|p| p["proposal_id"] == s.proposal_id)
                            .ok_or_else(|| ApiError::bad("planning_selection_invalid"))
                    })
                    .collect::<Result<_>>()?;
                let mut reviewed = input
                    .selected
                    .iter()
                    .map(|s| s.proposal.clone())
                    .collect::<Vec<_>>();
                for p in &mut reviewed {
                    p.depends_on = p
                        .depends_on
                        .iter()
                        .map(|index| {
                            selected_positions
                                .iter()
                                .position(|i| i == index)
                                .ok_or_else(|| ApiError::bad("planning_dependency_unselected"))
                        })
                        .collect::<Result<_>>()?;
                }
                Draft {
                    summary: "Reviewed proposals".into(),
                    tasks: reviewed,
                }
                .validate()
                .map_err(|_| ApiError::bad("planning_selection_invalid"))?;
                let new_ids: Vec<String> = input
                    .selected
                    .iter()
                    .map(|_| Uuid::new_v4().to_string())
                    .collect();
                let mut seen = std::collections::HashSet::new();
                let mut ids = Vec::new();
                let count: i64 = tx
                    .query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))
                    .map_err(internal)?;
                if count + input.selected.len() as i64 > 10000 {
                    return Err(ApiError::conflict("task_limit_reached"));
                }
                for (selected_index, selected) in input.selected.into_iter().enumerate() {
                    validate_proposal(&selected.proposal)
                        .map_err(|_| ApiError::bad("planning_selection_invalid"))?;
                    if !seen.insert(selected.proposal_id.clone())
                        || !v["proposals"].as_array().is_some_and(|a| {
                            a.iter().any(|p| p["proposal_id"] == selected.proposal_id)
                        })
                    {
                        return Err(ApiError::bad("planning_selection_invalid"));
                    }
                    let id = new_ids[selected_index].clone();
                    let at = now();
                    let p = selected.proposal;
                    let dependencies: Vec<String> = p
                        .depends_on
                        .iter()
                        .map(|index| {
                            new_ids[selected_positions.iter().position(|i| i == index).unwrap()]
                                .clone()
                        })
                        .collect();
                    let qa_policy = selected.qa_policy.unwrap_or_else(|| {
                        if p.task_type == "research" {
                            "skip".into()
                        } else {
                            "required".into()
                        }
                    });
                    if !["required", "skip"].contains(&qa_policy.as_str()) {
                        return Err(ApiError::bad("planning_selection_invalid"));
                    }
                    let task = json!({"id":id,"revision":1,"title":p.title,"objective":p.objective,"acceptance_criteria":p.acceptance_criteria,"priority":p.priority,"task_type":p.task_type,"labels":crate::tickets::labels(&p.labels)?,"dependencies":dependencies,"project":v["project"],"repository":v["repository"],"owner":"Coder","workflow":"engineering","status":"backlog","qa_policy":qa_policy,"merge_policy":"auto","automation_policy":"manual","automation_revision":null,"ready_at":null,"context":background(v["goal"].as_str().unwrap_or(""),v["context"].as_str().unwrap_or("")),"blocked_reason":null,"github_issue_url":null,"github_pull_request_url":null,"archived_at":null,"completed_at":null,"cancelled_at":null,"created_at":at,"updated_at":at,"source_plan":{"plan_id":v["id"],"proposal_id":selected.proposal_id,"plan_revision":input.expected_revision}});
                    if serde_json::to_vec(&task).map_err(internal)?.len() > 32768 {
                        return Err(ApiError::bad("planning_selection_invalid"));
                    }
                    crate::store::validate(&task, false, true)?;
                    persist(&tx, &task, "task.created")?;
                    ids.push(id);
                }
                v["accepted_task_ids"] = json!(ids);
                v["status"] = json!("accepted");
            } else if action == "discard" {
                if !body.as_object().is_some_and(|o| o.is_empty()) {
                    return Err(ApiError::bad("request_invalid"));
                }
                v["status"] = json!(if ["queued", "generating", "unknown", "cleanup_pending"]
                    .contains(&v["status"].as_str().unwrap_or(""))
                {
                    "cleanup_pending"
                } else if v["status"] == "accepted" {
                    return Err(ApiError::conflict("plan_already_accepted"));
                } else {
                    "discarded"
                });
            } else {
                return Err(ApiError::bad("request_invalid"));
            }
            v["revision"] = json!(v["revision"].as_u64().unwrap_or(0) + 1);
            v["updated_at"] = json!(now());
            save(&tx, &v)?;
            (200, json!({"plan":public(v)}))
        };
        tx.execute(
            "INSERT INTO commands VALUES(?1,?2,?3,?4)",
            params![key, fingerprint, status, response.to_string()],
        )
        .map_err(internal)?;
        tx.commit().map_err(internal)?;
        Ok((status, response))
    }
}
impl App {
    pub(crate) fn claim_plan(&self) -> Result<Option<Value>> {
        let mut s = self.inner.store.lock().map_err(|_| ApiError::internal())?;
        let tx =
            s.db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(internal)?;
        let raw:Option<String>=tx.query_row("SELECT document FROM plans WHERE json_extract(document,'$.status')='cleanup_pending' OR (json_extract(document,'$.status')='queued' AND (SELECT paused FROM execution WHERE singleton=1)=0) ORDER BY rowid LIMIT 1",[],|r|r.get(0)).optional().map_err(internal)?;
        let Some(raw) = raw else { return Ok(None) };
        let mut job: Value = serde_json::from_str(&raw).map_err(internal)?;
        job["cleanup_only"] = json!(job["status"] == "cleanup_pending");
        job["status"] = json!("generating");
        job["action_id"] = json!(Uuid::new_v4().to_string());
        if job["cleanup_only"] != true {
            job["generation_action_id"] = job["action_id"].clone();
        }
        job["revision"] = json!(job["revision"].as_u64().unwrap_or(0) + 1);
        save(&tx, &job)?;
        tx.commit().map_err(internal)?;
        Ok(Some(job))
    }
    pub(crate) async fn perform_plan(&self, mut job: Value) {
        let result = async {
            let f = self
                .inner
                .factory
                .as_ref()
                .ok_or_else(|| ApiError::unavailable("planning_unavailable"))?;
            let approval = f.approval(&job)?;
            let cleanup = job["cleanup_only"] == true;
            let receipt = self.invoke_plan(&approval, &job, cleanup).await?;
            if receipt.schema_version != 1
                || receipt.action_id != job["action_id"]
                || receipt.run_id != job["id"]
                || receipt.repository != job["repository"]
                || receipt.outcome != "ok"
                || !receipt.cleanup_complete
                || receipt.action != if cleanup { "cancel" } else { "plan" }
            {
                return Err(ApiError::bad("planning_result_invalid"));
            }
            if cleanup {
                return Ok(None);
            }
            if receipt.role.as_deref() != Some("plan")
                || !receipt.session_id.as_ref().is_some_and(|s| {
                    (8..=128).contains(&s.len())
                        && s.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_-:.".contains(&b))
                })
            {
                return Err(ApiError::bad("planning_result_invalid"));
            }
            let draft = receipt
                .plan
                .ok_or_else(|| ApiError::bad("planning_result_invalid"))?;
            draft.validate()?;
            Ok(Some(draft))
        }
        .await;
        if let Ok(store) = self.inner.store.lock() {
            let current = get(&store.db, job["id"].as_str().unwrap()).ok();
            if current
                .as_ref()
                .is_some_and(|v| v["status"] == "cleanup_pending")
            {
                self.changed_without_lock();
                return;
            }
            match result {
                Ok(Some(draft)) => {
                    job["status"] = json!("proposed");
                    job["summary"] = json!(draft.summary);
                    job["proposals"] = json!(
                        draft
                            .tasks
                            .into_iter()
                            .map(|p| {
                                let mut v = serde_json::to_value(p).unwrap();
                                v["proposal_id"] = json!(Uuid::new_v4().to_string());
                                v
                            })
                            .collect::<Vec<_>>()
                    );
                }
                Ok(None) => job["status"] = json!("discarded"),
                Err(e) => {
                    job["status"] = json!(if e.code() == "execution_paused" {
                        "queued"
                    } else {
                        "unknown"
                    });
                    job["reason"] = json!(e.code());
                }
            }
            job["revision"] = json!(job["revision"].as_u64().unwrap_or(0) + 1);
            job["updated_at"] = json!(now());
            let _ = save(&store.db, &job);
        }
        self.changed();
    }
    fn changed_without_lock(&self) {
        self.inner.changes.send_modify(|r| *r = r.wrapping_add(1));
        self.inner.wake.notify_one();
    }
    async fn invoke_plan(
        &self,
        approval: &Approval,
        job: &Value,
        cleanup: bool,
    ) -> Result<Receipt> {
        approval.verify(&approval.digest, cleanup)?;
        if !cleanup
            && !self
                .inner
                .factory
                .as_ref()
                .is_some_and(|f| f.planning_available())
        {
            return Err(ApiError::unavailable("planning_unavailable"));
        }
        if !cleanup && now_ms() >= job["deadline_ms"].as_i64().unwrap_or(0) {
            return Err(ApiError::unavailable("planning_deadline"));
        }
        let policy = &approval.policy;
        let adapter = policy
            .adapter_policy
            .as_ref()
            .ok_or_else(ApiError::internal)?;
        let req = json!({"schema_version":1,"action":if cleanup{"cancel"}else{"plan"},"action_id":job["action_id"],"run_id":job["id"],"repository":job["repository"],"task_snapshot":{"goal":job["goal"],"context":job["context"],"project":job["project"],"repository":job["repository"],"workflow":"engineering","merge_policy":"manual"},"deadline_ms":job["deadline_ms"],"stage_deadline_ms":if cleanup {now_ms()+30000}else{job["deadline_ms"].as_i64().unwrap_or(0)},"workspace":policy.work_root.join(job["id"].as_str().unwrap()),"base_sha":null,"head_sha":null,"base_ref":null,"qa_receipt":null,"merge_policy":"manual","observed_action_id":null,"approval_sha256":approval.digest,"adapter_policy":adapter});
        let mut cmd = Command::new(&policy.worker.executable);
        cmd.env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for name in &policy.worker.credential_env_names {
            if name == "CODEX_HOME" {
                cmd.env(name, &adapter.auth_home);
            } else if let Ok(value) = std::env::var(name) {
                cmd.env(name, value);
            }
        }
        #[cfg(unix)]
        cmd.process_group(0);
        let mut child = {
            let s = self.inner.store.lock().map_err(|_| ApiError::internal())?;
            if !cleanup
                && !self
                    .inner
                    .factory
                    .as_ref()
                    .is_some_and(|f| f.planning_available())
            {
                return Err(ApiError::unavailable("planning_unavailable"));
            }
            if !cleanup && s.execution_state()?["paused"] == true {
                return Err(ApiError::conflict("execution_paused"));
            }
            if !cleanup
                && (self.inner.stopping.load(Ordering::SeqCst)
                    || get(&s.db, job["id"].as_str().unwrap())?["status"] != "generating")
            {
                return Err(ApiError::unavailable("planning_interrupted"));
            }
            cmd.spawn()
                .map_err(|_| ApiError::unavailable("planning_transport_failed"))?
        };
        let pid = child.id().ok_or_else(ApiError::internal)?;
        let mut stdin = child.stdin.take().unwrap();
        let out = child.stdout.take().unwrap();
        let err = child.stderr.take().unwrap();
        let mut changes = self.inner.changes.subscribe();
        let transport = async {
            stdin
                .write_all(req.to_string().as_bytes())
                .await
                .map_err(internal)?;
            stdin.shutdown().await.map_err(internal)?;
            drop(stdin);
            let (status, out, _) = tokio::try_join!(
                async { child.wait().await.map_err(|_| ()) },
                read_bounded(out, policy.max_output_bytes),
                read_bounded(err, policy.max_output_bytes)
            )
            .map_err(|_| ApiError::unavailable("planning_transport_failed"))?;
            if !status.success() {
                return Err(ApiError::unavailable("planning_transport_failed"));
            }
            serde_json::from_slice(&out).map_err(|_| ApiError::bad("planning_result_invalid"))
        };
        let cancelled = async {
            loop {
                if !cleanup
                    && (self.inner.stopping.load(Ordering::SeqCst)
                        || self
                            .inner
                            .store
                            .lock()
                            .ok()
                            .and_then(|s| get(&s.db, job["id"].as_str().unwrap()).ok())
                            .is_some_and(|v| v["status"] == "cleanup_pending"))
                {
                    break;
                }
                let _ = changes.changed().await;
            }
        };
        let ms = if cleanup {
            30000
        } else {
            job["deadline_ms"]
                .as_i64()
                .unwrap_or(0)
                .saturating_sub(now_ms())
                .max(1)
        };
        let result = tokio::select! {r=tokio::time::timeout(Duration::from_millis(ms as u64),transport)=>match r {Ok(v)=>v,Err(_)=>Err(ApiError::unavailable("planning_deadline"))},_=cancelled=>Err(ApiError::unavailable("planning_interrupted"))};
        if !cleanup_group(&mut child, pid).await {
            return Err(ApiError::unavailable("planning_cleanup_unknown"));
        }
        result
    }
}

fn background(goal: &str, context: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for (label, text) in [
        ("Original goal (background)", goal),
        ("User context (background)", context),
    ] {
        let chars: Vec<char> = text.chars().collect();
        for chunk in chars.chunks(1900) {
            lines.push(format!("{label}: {}", chunk.iter().collect::<String>()));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proposal_byte_budget_and_unicode_background_fit_existing_task_lists() {
        let p = Proposal {
            title: "🦀".repeat(240),
            objective: "🦀".repeat(4000),
            acceptance_criteria: vec!["🦀".repeat(1000); 3],
            priority: 3,
            task_type: "feature".into(),
            labels: vec![],
            depends_on: vec![],
        };
        assert!(validate_proposal(&p).is_ok());
        assert!(
            Draft {
                summary: "Bounded review".into(),
                tasks: vec![p]
            }
            .validate()
            .is_err()
        );
        let goal = "🦀".repeat(3000);
        let chunks = background(&goal, "Keep the user constraints intact.");
        assert!(chunks.iter().all(|v| v.chars().count() <= 2000));
        assert_eq!(
            chunks
                .iter()
                .filter(|v| v.starts_with("Original goal"))
                .map(|v| v.split_once(": ").unwrap().1)
                .collect::<String>(),
            goal
        );
    }
}
