//! Repository-free research: durable reports, never Git delivery or task-selected files.
use crate::{ApiError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub fn eligible(task: &Value) -> bool {
    task["workflow"] == "engineering"
        && task["task_type"] == "research"
        && (task["repository"].is_null() || task["repository"].as_str() == Some(""))
}
pub fn is_run(run: &Value) -> bool {
    run["kind"] == "research"
}
fn invalid() -> ApiError {
    ApiError::bad("research_result_invalid")
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub title: String,
    pub url: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub markdown: String,
    pub sources: Vec<Source>,
}
impl Report {
    pub fn validate(&self) -> Result<()> {
        if self.markdown.trim().len() < 40
            || self.markdown.len() > 8000
            || self.sources.is_empty()
            || self.sources.len() > 8
            || serde_json::to_vec(self).map_err(|_| invalid())?.len() > 12000
        {
            return Err(invalid());
        }
        let mut seen = std::collections::HashSet::new();
        for source in &self.sources {
            let uri = source
                .url
                .parse::<axum::http::Uri>()
                .map_err(|_| invalid())?;
            if source.title.trim().is_empty()
                || source.title.len() > 160
                || source.url.len() > 1000
                || source.url.contains(['\\', '\r', '\n'])
                || !matches!(uri.scheme_str(), Some("http" | "https"))
                || uri.authority().is_none_or(|a| a.as_str().contains('@'))
                || !seen.insert(&source.url)
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<String> {
        self.validate()?;
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).map_err(|_| invalid())?)
        ))
    }
}
pub fn findings(values: &[String], passed: bool) -> Result<()> {
    if values.len() > 4
        || values.iter().any(|s| s.trim().is_empty() || s.len() > 1000)
        || (passed && !values.is_empty())
        || (!passed && values.is_empty())
    {
        return Err(invalid());
    }
    Ok(())
}
pub fn schema(qa: bool) -> Value {
    if qa {
        json!({"type":"object","additionalProperties":false,"required":["passed","findings"],"properties":{"passed":{"type":"boolean"},"findings":{"type":"array","items":{"type":"string"}}}})
    } else {
        json!({"type":"object","additionalProperties":false,"required":["markdown","sources"],"properties":{"markdown":{"type":"string"},"sources":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["title","url"],"properties":{"title":{"type":"string"},"url":{"type":"string"}}}}}})
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Review {
    pub passed: bool,
    pub findings: Vec<String>,
}
/// Actual exec JSONL events, not text quoted inside an agent message.
pub(crate) fn parse_events(bytes: &[u8], qa: bool) -> Result<(String, Value, u32)> {
    let (mut session, mut message, mut complete, mut searches) = (None, None, false, 0u32);
    for line in bytes.split(|b| *b == b'\n').filter(|v| !v.is_empty()) {
        let event: Value = serde_json::from_slice(line).map_err(|_| invalid())?;
        match event["type"].as_str().unwrap_or("") {
            "thread.started" => session = event["thread_id"].as_str().map(String::from),
            "turn.completed" => complete = true,
            "turn.failed" | "error" => return Err(invalid()),
            "item.started"
                if !matches!(
                    event["item"]["type"].as_str(),
                    Some("web_search" | "agent_message" | "reasoning" | "todo_list")
                ) =>
            {
                return Err(invalid());
            }
            "item.completed" => match event["item"]["type"].as_str().unwrap_or("") {
                "web_search" => searches += 1,
                "agent_message" => message = event["item"]["text"].as_str().map(String::from),
                "reasoning" | "todo_list" => {}
                _ => return Err(invalid()),
            },
            _ => {}
        }
    }
    let session = session
        .filter(|s| {
            (8..=128).contains(&s.len())
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-:.".contains(&b))
        })
        .ok_or_else(invalid)?;
    if !complete || searches == 0 {
        return Err(invalid());
    }
    let value: Value =
        serde_json::from_str(&message.ok_or_else(invalid)?).map_err(|_| invalid())?;
    if qa {
        let review: Review = serde_json::from_value(value.clone()).map_err(|_| invalid())?;
        findings(&review.findings, review.passed)?;
    } else {
        let report: Report = serde_json::from_value(value.clone()).map_err(|_| invalid())?;
        report.validate()?;
    }
    Ok((session, value, searches))
}

pub(crate) fn admit(
    tx: &rusqlite::Transaction<'_>,
    task: &Value,
    body: &Value,
    approval: Option<&crate::execution_config::Approval>,
) -> Result<(u16, Value)> {
    use crate::store::{internal, now, persist};
    use rusqlite::params;
    let approval = approval.ok_or_else(|| ApiError::unavailable("research_unavailable"))?;
    approval.verify(&approval.digest, false)?;
    if approval
        .policy
        .adapter_policy
        .as_ref()
        .and_then(|p| p.agent_config.as_ref())
        .is_none_or(|c| !crate::system::get(tx).is_ok_and(|r| r.revision == c.revision))
    {
        return Err(ApiError::conflict("agent_config_changed"));
    }
    if !approval
        .policy
        .adapter_policy
        .as_ref()
        .is_some_and(|p| p.research)
    {
        return Err(ApiError::unavailable("research_unavailable"));
    }
    if body.as_object().is_none_or(|o| o.len() != 1)
        || body["expected_revision"] != task["revision"]
    {
        return Err(ApiError::conflict("revision_conflict"));
    }
    if task["status"] != "ready"
        || !task["blocked_reason"].is_null()
        || !task["archived_at"].is_null()
    {
        return Err(ApiError::conflict("task_not_runnable"));
    }
    crate::tickets::validate_size(task)?;
    if !crate::tickets::blockers(tx, task)?.is_empty() {
        return Err(ApiError::conflict("dependencies_unmet"));
    }
    if tx
        .query_row("SELECT paused FROM execution WHERE singleton=1", [], |r| {
            r.get::<_, bool>(0)
        })
        .map_err(internal)?
    {
        return Err(ApiError::conflict("execution_paused"));
    }
    let busy:i64=tx.query_row("SELECT count(*) FROM runs WHERE json_extract(document,'$.kind')='research' AND json_extract(document,'$.status') NOT IN ('succeeded','failed','cancelled')",[],|r|r.get(0)).map_err(internal)?;
    if busy > 0 {
        return Err(ApiError::conflict("active_run_exists"));
    }
    let prior:i64=tx.query_row("SELECT count(*) FROM runs WHERE task_id=?1 AND json_extract(document,'$.task_revision')=?2",params![task["id"].as_str(),task["revision"].as_u64()],|r|r.get(0)).map_err(internal)?;
    if prior > 0 {
        return Err(ApiError::conflict("automatic_run_already_consumed"));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let workspace = approval.policy.work_root.join(&id);
    let run = json!({"id":id,"kind":"research","task_id":task["id"],"task_revision":task["revision"],"task_snapshot":task,"repository":"","status":"admitted","stage":"research","approval_sha256":approval.digest,"runtime_approval_path":approval.path(),"workspace":workspace,"agents":approval.policy.adapter_policy.as_ref().and_then(|p|p.agent_config.as_ref()).map(|c|c.public()),"created_at":now(),"updated_at":now(),"deadline_ms":crate::execution_config::now_ms()+approval.policy.max_run_seconds as i64*1000,"receipt_count":0,"repair_count":0,"max_qa_repairs":approval.policy.max_qa_repairs.unwrap_or(2),"session_history":[],"review_history":[],"automation":{"revision":task["revision"]}});
    crate::execution_store::save(tx, &run)?;
    let mut active = task.clone();
    active["status"] = json!("active");
    active["revision"] = json!(task["revision"].as_u64().unwrap() + 1);
    active["updated_at"] = json!(now());
    persist(tx, &active, "task.updated")?;
    crate::execution_store::event(tx, &run, "run.created")?;
    Ok((202, json!({"run":crate::execution_store::public_run(run)})))
}
impl crate::App {
    pub(crate) fn pick_research(&self) -> Result<bool> {
        let Some(factory) = &self.inner.factory else {
            return Ok(false);
        };
        if !factory.research_available() {
            return Ok(false);
        }
        let mut store = self.inner.store.lock().map_err(|_| ApiError::internal())?;
        if store.execution_state()?["paused"] != false {
            return Ok(false);
        }
        if store.runs(None)?.iter().any(|r| {
            is_run(r) && !crate::execution_store::terminal(r["status"].as_str().unwrap_or(""))
        }) {
            return Ok(false);
        }
        let snapshot = store.snapshot()?;
        let mut tasks = snapshot["tasks"].as_array().cloned().unwrap_or_default();
        tasks.sort_by_key(|t| {
            (
                match t["priority"].as_u64().unwrap_or(3) {
                    0 => 5,
                    n => n,
                },
                t["ready_at"].as_str().unwrap_or("").to_string(),
                t["id"].as_str().unwrap_or("").to_string(),
            )
        });
        let registry = crate::system::get(&store.db)?;
        for task in tasks {
            if !eligible(&task)
                || task["status"] != "ready"
                || task["automation_policy"] != "ready_to_done"
                || task["automation_revision"] != task["revision"]
                || !crate::tickets::blockers(&store.db, &task)?.is_empty()
            {
                continue;
            }
            let approval = factory
                .research_approval(task["id"].as_str().unwrap())?
                .with_agents(registry.assignment(Some(&task))?)?;
            let key = format!("research:{}:{}", task["id"], task["revision"]);
            match store.execution_command_guarded(
                &key,
                &key,
                "run",
                task["id"].as_str(),
                json!({"expected_revision":task["revision"]}),
                crate::execution_store::Admission {
                    approval: Some(&approval),
                    agent_registry: Some(&registry),
                    ..Default::default()
                },
            ) {
                Ok(_) => return Ok(true),
                Err(e)
                    if matches!(
                        e.code(),
                        "active_run_exists"
                            | "automatic_run_already_consumed"
                            | "task_not_runnable"
                    ) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(false)
    }
}
pub(crate) async fn perform(
    app: &crate::App,
    approval: &crate::execution_config::Approval,
    run: &Value,
    attempt: &Value,
) -> Result<(Value, Option<Value>)> {
    use crate::execution::{DriverOutcome, driver};
    use crate::execution_config::now_ms;
    let action = attempt["action"].as_str().ok_or_else(invalid)?;
    let cleanup = matches!(action, "cancel" | "observe");
    let wire_action = if cleanup { "cancel" } else { action };
    let request = json!({"schema_version":1,"action":wire_action,"action_id":attempt["id"],"run_id":run["id"],"repository":"","task_snapshot":run["task_snapshot"],"deadline_ms":run["deadline_ms"],"stage_deadline_ms":if cleanup {now_ms()+30000}else{run["deadline_ms"].as_i64().unwrap_or(0).min(now_ms()+approval.policy.max_stage_seconds as i64*1000)},"workspace":run["workspace"],"base_sha":null,"head_sha":null,"base_ref":null,"qa_receipt":null,"merge_policy":run["task_snapshot"]["merge_policy"],"observed_action_id":run["last_effect_attempt"],"approval_sha256":run["approval_sha256"],"adapter_policy":approval.policy.adapter_policy,"research_report":run["report"],"research_findings":run["research_findings"]});
    if request.to_string().len() > 65536 {
        return Err(invalid());
    }
    let receipt = match driver(
        app,
        approval,
        &approval.policy.worker,
        run,
        if cleanup { "cancel" } else { action },
        &request,
    )
    .await?
    {
        DriverOutcome::Completed(r) => *r,
        other => {
            let mut next = run.clone();
            match other {
                DriverOutcome::StoppedBeforeSpawn => next["status"] = json!("cancel_requested"),
                DriverOutcome::ShutdownBeforeSpawn => {
                    next["status"] = json!("unknown");
                    next["reason"] = json!("driver_interrupted");
                }
                _ => {}
            }
            return Ok((next, None));
        }
    };
    if receipt.schema_version != 1
        || receipt.action_id != attempt["id"]
        || receipt.run_id != run["id"]
        || receipt.action != wire_action
        || !receipt.repository.is_empty()
        || receipt.outcome != "ok"
        || !receipt.cleanup_complete
        || receipt.base_sha.is_some()
        || receipt.head_sha.is_some()
        || receipt.base_ref.is_some()
        || receipt.reviewed_sha.is_some()
        || receipt.pull_request_url.is_some()
        || receipt.merged.is_some()
        || receipt.merge_commit_sha.is_some()
    {
        return Err(invalid());
    }
    let mut next = run.clone();
    if cleanup {
        next["status"] = json!("cancelled");
        next["reason"] = json!("cleanup_confirmed");
        return Ok((
            next,
            Some(serde_json::to_value(receipt).map_err(|_| invalid())?),
        ));
    }
    let session = receipt
        .session_id
        .as_deref()
        .filter(|s| {
            (8..=128).contains(&s.len())
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-:.".contains(&b))
        })
        .ok_or_else(invalid)?;
    let mut seen = run["session_history"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if seen.iter().any(|s| s == session) || receipt.search_count.unwrap_or(0) == 0 {
        return Err(invalid());
    }
    seen.push(json!(session));
    next["session_history"] = json!(seen);
    if action == "research" {
        if receipt.role.as_deref() != Some("coder") || receipt.passed != Some(true) {
            return Err(invalid());
        }
        let report = receipt.report.as_ref().ok_or_else(invalid)?;
        let digest = report.digest()?;
        if receipt.report_digest.as_deref() != Some(&digest) {
            return Err(invalid());
        }
        next["report"] = json!(report);
        next["report_digest"] = json!(digest);
        next["coder_session"] = json!(session);
        next["research_findings"] = Value::Null;
        if run["task_snapshot"]["qa_policy"] == "skip" {
            next["status"] = json!("succeeded");
            next["stage"] = json!("report_ready");
        } else {
            next["stage"] = json!("research_qa");
        }
    } else if action == "research_qa" {
        if receipt.role.as_deref() != Some("qa")
            || receipt.report_digest.as_deref() != run["report_digest"].as_str()
        {
            return Err(invalid());
        }
        let passed = receipt.passed.ok_or_else(invalid)?;
        let issues = receipt.research_findings.as_ref().ok_or_else(invalid)?;
        findings(issues, passed)?;
        let mut history = run["review_history"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        history.push(json!({"report_digest":run["report_digest"],"passed":passed,"findings":issues,"at":crate::store::now()}));
        next["review_history"] = json!(history);
        next["qa_receipt"] = json!(receipt);
        if passed {
            next["status"] = json!("succeeded");
            next["stage"] = json!("report_ready");
        } else if run["repair_count"].as_u64().unwrap_or(0)
            < run["max_qa_repairs"].as_u64().unwrap_or(0)
        {
            next["repair_count"] = json!(run["repair_count"].as_u64().unwrap_or(0) + 1);
            next["research_findings"] = json!(issues);
            next["stage"] = json!("research");
            next["reason"] = json!("research_repair_queued");
        } else {
            next["status"] = json!("failed");
            next["reason"] = json!("research_qa_exhausted");
        }
    } else {
        return Err(invalid());
    }
    Ok((
        next,
        Some(serde_json::to_value(receipt).map_err(|_| invalid())?),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_bounds_urls_and_real_search_event_gates() {
        let report = json!({"markdown":"A substantive synthetic finding with clear evidence and limitations.","sources":[{"title":"Primary","url":"https://example.org/source"}]});
        let events = |tool: Option<&str>, search: bool, done: bool| {
            let mut lines = vec![json!({"type":"thread.started","thread_id":"research-session"})];
            if let Some(kind) = tool {
                lines.push(json!({"type":"item.started","item":{"type":kind}}));
            }
            if search {
                lines.push(json!({"type":"item.completed","item":{"type":"web_search","query":"synthetic"}}));
            }
            lines.push(json!({"type":"item.completed","item":{"type":"agent_message","text":report.to_string()}}));
            if done {
                lines.push(json!({"type":"turn.completed"}));
            }
            lines
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                .into_bytes()
        };
        assert!(parse_events(&events(None, true, true), false).is_ok());
        for (kind, search, done) in [
            (None, false, true),
            (None, true, false),
            (Some("command_execution"), true, true),
            (Some("mcp_tool_call"), true, true),
        ] {
            assert!(parse_events(&events(kind, search, done), false).is_err());
        }
        let credentials = format!("https://{}@example.org", "synthetic:invalid");
        for url in [
            "javascript:alert(1)",
            credentials.as_str(),
            "file:///etc/passwd",
            "https://example.org/\n",
        ] {
            let mut r: Report = serde_json::from_value(report.clone()).unwrap();
            r.sources[0].url = url.into();
            assert!(r.validate().is_err());
        }
        let mut r: Report = serde_json::from_value(report).unwrap();
        r.markdown = "x".repeat(8001);
        assert!(r.validate().is_err());
    }
    #[test]
    fn acknowledged_research_stop_wins_over_completed_report_before_commit() {
        use crate::store::Store;
        use rusqlite::params;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("board.sqlite");
        let mut store = Store::open(&path).unwrap();
        let task = store
            .command(
                "task",
                "task",
                None,
                "create",
                json!({"title":"Synthetic","objective":"Research Stop race","workflow":"manual"}),
            )
            .unwrap()
            .1["task"]
            .clone();
        let id = uuid::Uuid::new_v4().to_string();
        let aid = uuid::Uuid::new_v4().to_string();
        let current = json!({"id":id,"task_id":task["id"],"kind":"research","status":"cancel_requested","stage":"research","last_attempt":aid,"receipt_count":0});
        crate::execution_store::save(&store.db, &current).unwrap();
        store
            .db
            .execute(
                "INSERT INTO attempts VALUES(?1,?2,?3)",
                params![
                    aid,
                    id,
                    json!({"id":aid,"run_id":id,"action":"research","state":"attempted"})
                        .to_string()
                ],
            )
            .unwrap();
        let mut completed = current;
        completed["status"] = json!("succeeded");
        completed["stage"] = json!("report_ready");
        store
            .finish_attempt(&id, &aid, completed, Some(json!({"cleanup_complete":true})))
            .unwrap();
        assert_eq!(
            store.current_run(&id).unwrap()["status"],
            "cancel_requested"
        );
        let (_, cleanup) = store.next_attempt().unwrap().unwrap();
        assert_eq!(cleanup["action"], "cancel");
        assert_eq!(cleanup["driver"], "worker");
    }
}
