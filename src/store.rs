use crate::{ApiError, Result};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};
use uuid::Uuid;

pub struct Store {
    pub(crate) db: Connection,
    _lock: std::fs::File,
}
pub(crate) fn internal(_: impl std::fmt::Display) -> ApiError {
    ApiError::internal()
}
pub(crate) fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        crate::storage_ownership::validate(path, false)?;
        let lock = crate::storage_ownership::lock(path)?;
        crate::storage_ownership::private_options()
            .create(true)
            .open(path)
            .map_err(internal)?;
        let db = Connection::open(path).map_err(internal)?;
        db.busy_timeout(Duration::from_millis(250))
            .map_err(internal)?;
        let application: i64 = db
            .query_row("PRAGMA application_id", [], |r| r.get(0))
            .map_err(internal)?;
        let tables: i64 = db
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )
            .map_err(internal)?;
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(internal)?;
        if (application != 0 && application != 0x57425253)
            || ((tables > 0 || version != 0) && application != 0x57425253)
            || (version != 0 && version != 1 && version != 2)
        {
            return Err(ApiError::bad("database_not_workboard_rs"));
        }
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
            BEGIN IMMEDIATE;
            CREATE TABLE IF NOT EXISTS tasks(id TEXT PRIMARY KEY, document TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS events(cursor INTEGER PRIMARY KEY AUTOINCREMENT, task_id TEXT REFERENCES tasks(id), document TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS events_task ON events(task_id,cursor);
            CREATE TABLE IF NOT EXISTS commands(key TEXT PRIMARY KEY, fingerprint TEXT NOT NULL, status INTEGER NOT NULL, response TEXT NOT NULL);
            PRAGMA application_id=1463964243;
            COMMIT;").map_err(internal)?;
        if version == 1 {
            db.execute_batch("BEGIN IMMEDIATE; ALTER TABLE events RENAME TO events_v1; DROP INDEX events_task; CREATE TABLE events(cursor INTEGER PRIMARY KEY AUTOINCREMENT,task_id TEXT REFERENCES tasks(id),document TEXT NOT NULL); INSERT INTO events SELECT * FROM events_v1; DROP TABLE events_v1; CREATE INDEX events_task ON events(task_id,cursor); COMMIT;").map_err(internal)?;
        }
        db.execute_batch("BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS runs(id TEXT PRIMARY KEY,task_id TEXT NOT NULL REFERENCES tasks(id),document TEXT NOT NULL); CREATE INDEX IF NOT EXISTS runs_task ON runs(task_id); CREATE TABLE IF NOT EXISTS attempts(id TEXT PRIMARY KEY,run_id TEXT NOT NULL REFERENCES runs(id),document TEXT NOT NULL); CREATE TABLE IF NOT EXISTS projects(id TEXT PRIMARY KEY,repository TEXT NOT NULL UNIQUE,local_path TEXT NOT NULL UNIQUE,document TEXT NOT NULL); CREATE UNIQUE INDEX IF NOT EXISTS projects_repository_identity ON projects(json_extract(document,'$.repository_id')) WHERE json_extract(document,'$.repository_id') IS NOT NULL; CREATE TABLE IF NOT EXISTS project_lanes(project_id TEXT PRIMARY KEY REFERENCES projects(id),run_id TEXT NOT NULL UNIQUE REFERENCES runs(id),repository TEXT NOT NULL UNIQUE); CREATE TABLE IF NOT EXISTS automation_consumptions(activation_sha256 TEXT PRIMARY KEY,run_id TEXT NOT NULL REFERENCES runs(id)); CREATE TABLE IF NOT EXISTS execution(singleton INTEGER PRIMARY KEY CHECK(singleton=1),paused INTEGER NOT NULL,lane_run_id TEXT); INSERT OR IGNORE INTO execution VALUES(1,1,NULL); PRAGMA user_version=2; COMMIT;").map_err(internal)?;
        db.execute("UPDATE projects SET document=json_set(document,'$.paused',json('true'),'$.runtime_status','failed','$.failure','project_discovery_interrupted') WHERE json_extract(document,'$.runtime_status')='preparing'",[]).map_err(internal)?;
        db.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS tasks_qa_bug_fingerprint ON tasks(json_extract(document,'$.qa_bug_fingerprint')) WHERE json_extract(document,'$.qa_bug_fingerprint') IS NOT NULL;").map_err(internal)?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS plans(id TEXT PRIMARY KEY,document TEXT NOT NULL); UPDATE plans SET document=json_set(document,'$.status','unknown','$.reason','planning_interrupted','$.revision',json_extract(document,'$.revision')+1) WHERE json_extract(document,'$.status') IN ('queued','generating','cleanup_pending');").map_err(internal)?;
        let mut store = Self { db, _lock: lock };
        store.recover_runs()?;
        Ok(store)
    }
    pub fn snapshot(&self) -> Result<Value> {
        // A single connection is serialized by the caller; each snapshot is internally consistent.
        let tx = self.db.unchecked_transaction().map_err(internal)?;
        let mut query = self
            .db
            .prepare("SELECT document FROM tasks ORDER BY rowid DESC")
            .map_err(internal)?;
        let rows = query
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(internal)?;
        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(crate::tickets::project(
                &self.db,
                serde_json::from_str::<Value>(&row.map_err(internal)?).map_err(internal)?,
            )?);
        }
        let runs = self.runs(None)?;
        let execution = self.execution_state()?;
        let watermark: i64 = self
            .db
            .query_row("SELECT COALESCE(MAX(cursor),0) FROM events", [], |r| {
                r.get(0)
            })
            .map_err(internal)?;
        let events = self.events(0, None, true)?;
        drop(query);
        tx.commit().map_err(internal)?;
        Ok(
            json!({"schema_version":1,"projects":self.projects()?.into_iter().map(crate::projects::public).collect::<Vec<_>>(),"tasks":tasks,"plans":crate::planning::list(&self.db)?,"runs":runs,"execution":execution,"events":events,"watermark":watermark,"execution_activity":{"state":"available","tasks":{}}}),
        )
    }
    pub fn task(&self, id: &str) -> Result<Value> {
        crate::tickets::project(&self.db, get(&self.db, id)?)
    }
    pub fn events(&self, after: i64, task_id: Option<&str>, latest: bool) -> Result<Vec<Value>> {
        let order = if latest { "DESC" } else { "ASC" };
        let sql = format!(
            "SELECT cursor,document FROM events WHERE cursor > ?1 AND (?2 IS NULL OR task_id=?2) ORDER BY cursor {order} LIMIT 200"
        );
        let mut query = self.db.prepare(&sql).map_err(internal)?;
        let rows = query
            .query_map(params![after, task_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(internal)?;
        let mut events = Vec::new();
        for row in rows {
            let (cursor, doc) = row.map_err(internal)?;
            let mut value: Value = serde_json::from_str(&doc).map_err(internal)?;
            value["cursor"] = json!(cursor);
            events.push(value);
        }
        if latest {
            events.reverse();
        }
        Ok(events)
    }
    #[cfg(test)]
    pub fn command(
        &mut self,
        key: &str,
        fingerprint: &str,
        id: Option<&str>,
        action: &str,
        input: Value,
    ) -> Result<(u16, Value)> {
        self.command_guarded(
            key,
            fingerprint,
            id,
            action,
            input,
            crate::execution_store::Admission::default(),
        )
    }
    pub fn command_guarded(
        &mut self,
        key: &str,
        fingerprint: &str,
        id: Option<&str>,
        action: &str,
        input: Value,
        admission: crate::execution_store::Admission<'_>,
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
        let fields = input
            .as_object()
            .ok_or_else(|| ApiError::bad("request_invalid"))?;
        let creating = id.is_none();
        if let Some(task_id) = id {
            let active:i64=tx.query_row("SELECT COUNT(*) FROM runs WHERE task_id=?1 AND json_extract(document,'$.status') NOT IN ('succeeded','failed','cancelled')",[task_id],|r|r.get(0)).map_err(internal)?;
            if active > 0 {
                return Err(ApiError::conflict("task_has_active_run"));
            }
        }
        let mut task = if let Some(id) = id {
            get(&tx, id)?
        } else {
            let count: i64 = tx
                .query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))
                .map_err(internal)?;
            if count >= 10000 {
                return Err(ApiError::conflict("task_limit_reached"));
            }
            json!({"id":Uuid::new_v4().to_string(),"revision":0,"title":"","objective":"","acceptance_criteria":[],"context":[],"project":null,"owner":if admission.repository_queue && !fields.contains_key("owner") {json!("Coder")} else {Value::Null},"priority":3,"status":"backlog","workflow":if admission.repository_queue && input["workflow"].is_null() {"engineering"} else {"manual"},"repository":null,"qa_policy":"required","merge_policy":if input["workflow"]=="engineering" || (admission.repository_queue && input["workflow"].is_null()) {"auto"} else {"manual"},"automation_policy":"manual","blocked_reason":null,"github_issue_url":null,"github_pull_request_url":null,"archived_at":null,"completed_at":null,"cancelled_at":null,"created_at":now()})
        };
        if !creating {
            let expected = fields
                .get("expected_revision")
                .and_then(Value::as_u64)
                .filter(|v| *v > 0)
                .ok_or_else(|| ApiError::conflict("revision_required"))?;
            if task["revision"].as_u64() != Some(expected) {
                return Err(ApiError::conflict("revision_conflict"));
            }
        }
        if creating || action == "edit" {
            if !creating
                && (!task["archived_at"].is_null()
                    || ["done", "cancelled"].contains(&task["status"].as_str().unwrap_or("")))
            {
                return Err(ApiError::conflict("task_closed"));
            }
            let allowed = [
                "title",
                "objective",
                "acceptance_criteria",
                "context",
                "project",
                "project_id",
                "owner",
                "coding_agent_id",
                "priority",
                "criticality",
                "task_type",
                "labels",
                "dependencies",
                "status",
                "workflow",
                "repository",
                "qa_policy",
                "merge_policy",
                "automation_policy",
                "blocked_reason",
                "github_issue_url",
                "github_pull_request_url",
            ];
            for (field, value) in fields {
                if field == "expected_revision" && !creating {
                    continue;
                }
                if !allowed.contains(&field.as_str()) {
                    return Err(ApiError::bad("request_invalid"));
                }
                task[field] = value.clone();
            }
        } else {
            let extra: Vec<_> = fields
                .keys()
                .filter(|k| {
                    k.as_str() != "expected_revision"
                        && !(action == "block" && k.as_str() == "blocked_reason")
                })
                .collect();
            if !extra.is_empty() {
                return Err(ApiError::bad("request_invalid"));
            }
            match action {
                "accept" => {
                    if task["workflow"] != "manual" {
                        return Err(ApiError::unavailable("execution_unavailable"));
                    }
                    if !task["archived_at"].is_null()
                        || !task["blocked_reason"].is_null()
                        || !["backlog", "ready", "review", "active"]
                            .contains(&task["status"].as_str().unwrap_or(""))
                    {
                        return Err(ApiError::conflict("task_not_acceptable"));
                    }
                    task["status"] = json!("done");
                }
                "archive" => {
                    if !["done", "cancelled"].contains(&task["status"].as_str().unwrap_or("")) {
                        return Err(ApiError::conflict("task_not_closed"));
                    }
                    task["archived_at"] = json!(now());
                }
                "restore" => {
                    if task["archived_at"].is_null() {
                        return Err(ApiError::conflict("task_not_archived"));
                    }
                    task["archived_at"] = Value::Null;
                }
                "reopen" => {
                    if !["done", "cancelled"].contains(&task["status"].as_str().unwrap_or("")) {
                        return Err(ApiError::conflict("task_not_closed"));
                    }
                    task["status"] = json!("backlog");
                    task["archived_at"] = Value::Null;
                    task["blocked_reason"] = Value::Null;
                }
                "block" => {
                    if !task["archived_at"].is_null()
                        || ["done", "cancelled"].contains(&task["status"].as_str().unwrap_or(""))
                    {
                        return Err(ApiError::conflict("task_closed"));
                    }
                    task["blocked_reason"] =
                        fields.get("blocked_reason").cloned().unwrap_or(Value::Null);
                }
                _ => return Err(ApiError::bad("request_invalid")),
            }
        }
        if !creating
            && get(&tx, id.unwrap())?["workflow"] == "engineering"
            && task["workflow"] != "engineering"
        {
            return Err(ApiError::conflict("workflow_locked"));
        }
        if !task["project_id"].is_null()
            && !task["project_id"]
                .as_str()
                .is_some_and(|id| Uuid::parse_str(id).is_ok())
        {
            return Err(ApiError::bad("project_not_found"));
        }
        if !creating
            && get(&tx, id.unwrap())?["project_id"].is_string()
            && get(&tx, id.unwrap())?["project_id"] != task["project_id"]
        {
            return Err(ApiError::conflict("project_binding_locked"));
        }
        if let Some(project_id) = task["project_id"].as_str() {
            let project = crate::projects::get(&tx, project_id)?;
            if task["workflow"] != "engineering" {
                return Err(ApiError::bad("project_requires_engineering"));
            }
            if fields.contains_key("repository") && task["repository"] != project["repository"] {
                return Err(ApiError::bad("project_repository_mismatch"));
            }
            task["repository"] = project["repository"].clone();
            if task["project"].is_null() {
                task["project"] = project["name"].clone();
            }
            task["project_repository_id"] = project["repository_id"].clone();
            task["project_default_branch"] = project["default_branch"].clone();
            if input["automation_policy"] == "ready_to_done" {
                let approved = admission
                    .approval
                    .ok_or_else(|| ApiError::unavailable("project_runtime_unavailable"))?;
                approved.verify(project["approval_sha256"].as_str().unwrap_or(""), false)?;
            }
        }
        if !creating
            && !get(&tx, id.unwrap())?["qa_source"].is_null()
            && get(&tx, id.unwrap())?["repository"] != task["repository"]
        {
            return Err(ApiError::conflict("qa_bug_repository_locked"));
        }
        let enroll = input["automation_policy"] == "ready_to_done"
            || (admission.repository_queue
                && task["project_id"].is_null()
                && task["workflow"] == "engineering"
                && input["automation_policy"].is_null()
                && (creating || input["status"] == "ready"));
        if enroll {
            task["automation_policy"] = json!("ready_to_done");
        }
        if (creating || action == "edit")
            && enroll
            && task["project_id"].is_null()
            && !admission.repository_queue
        {
            let activation = admission
                .automatic
                .ok_or_else(|| ApiError::unavailable("automation_unavailable"))?;
            let approval = admission
                .approval
                .ok_or_else(|| ApiError::unavailable("automation_unavailable"))?;
            activation.verify(approval)?;
            let guard = admission
                .ownership
                .ok_or_else(|| ApiError::unavailable("automation_unavailable"))?;
            guard
                .lock()
                .map_err(|_| ApiError::internal())?
                .assert(task["repository"].as_str().unwrap_or(""))?;
            if task["repository"] != activation.repository() {
                return Err(ApiError::conflict("automation_scope_mismatch"));
            }
            let spent: i64 = tx
                .query_row(
                    "SELECT count(*) FROM automation_consumptions WHERE activation_sha256=?1",
                    [&activation.digest],
                    |r| r.get(0),
                )
                .map_err(internal)?;
            if spent > 0 {
                return Err(ApiError::conflict("automatic_run_already_consumed"));
            }
        }
        crate::tickets::normalize(&mut task)?;
        if creating && !fields.contains_key("qa_policy") && task["task_type"] == "research" {
            task["qa_policy"] = json!("skip");
        }
        crate::tickets::validate_dependencies(&tx, &task)?;
        let preserve_done = !creating
            && ["archive", "restore"].contains(&action)
            && get(&tx, id.unwrap())?["status"] == "done";
        if !(creating || action == "edit") || (!fields.contains_key("automation_policy") && !enroll)
        {
            task["automation_policy"] = json!("manual");
            task["automation_revision"] = Value::Null;
        }
        if !task["coding_agent_id"].is_null() && !task["coding_agent_id"].is_string() {
            return Err(ApiError::bad("coding_agent_unavailable"));
        }
        if admission.agent_registry.is_some() {
            let registry = crate::system::get(&tx)?;
            if task["workflow"] == "engineering" {
                if creating
                    || (action == "edit"
                        && fields.contains_key("coding_agent_id")
                        && get(&tx, id.unwrap())?["coding_agent_id"] != task["coding_agent_id"])
                {
                    task["coding_agent_id"] =
                        json!(registry.coder(task["coding_agent_id"].as_str())?.id);
                }
            } else {
                task["coding_agent_id"] = Value::Null;
            }
        } else if let Some(native) = admission.coding_agents {
            if task["workflow"] == "engineering" {
                let agent = native.agent(task["coding_agent_id"].as_str(), false)?;
                task["coding_agent_id"] = json!(agent.id);
            } else {
                task["coding_agent_id"] = Value::Null;
            }
        } else if fields.contains_key("coding_agent_id") && !task["coding_agent_id"].is_null() {
            return Err(ApiError::bad("coding_agent_unavailable"));
        }
        if crate::research::eligible(&task) {
            task["repository"] = Value::Null;
        }
        validate(&task, preserve_done, admission.repository_queue)?;
        let prior_ready = !creating && get(&tx, id.unwrap())?["status"] == "ready";
        if task["status"] == "ready" {
            if !prior_ready || task["ready_at"].is_null() {
                task["ready_at"] = json!(now());
            }
        } else {
            task["ready_at"] = Value::Null;
        }
        task["revision"] = json!(task["revision"].as_u64().unwrap_or(0) + 1);
        if (creating || action == "edit") && enroll {
            task["automation_revision"] = task["revision"].clone();
        } else {
            task["automation_policy"] = json!("manual");
            task["automation_revision"] = Value::Null;
        }

        task["updated_at"] = json!(now());
        task["completed_at"] = if task["status"] == "done" {
            if task["completed_at"].is_null() {
                task["updated_at"].clone()
            } else {
                task["completed_at"].clone()
            }
        } else {
            Value::Null
        };
        task["cancelled_at"] = if task["status"] == "cancelled" {
            if task["cancelled_at"].is_null() {
                task["updated_at"].clone()
            } else {
                task["cancelled_at"].clone()
            }
        } else {
            Value::Null
        };
        let event_type = if creating {
            "task.created"
        } else if action == "edit" {
            "task.updated"
        } else {
            match action {
                "accept" => "task.accepted",
                "archive" => "task.archived",
                "restore" => "task.restored",
                "reopen" => "task.reopened",
                _ => "task.updated",
            }
        };
        if creating || action == "edit" {
            crate::tickets::validate_size(&task)?;
        }
        persist(&tx, &task, event_type)?;
        let response = json!({"task":crate::tickets::project(&tx,task)?});
        let status = if creating { 201 } else { 200 };
        tx.execute(
            "INSERT INTO commands(key,fingerprint,status,response) VALUES(?1,?2,?3,?4)",
            params![key, fingerprint, status, response.to_string()],
        )
        .map_err(internal)?;
        tx.commit().map_err(internal)?;
        Ok((status, response))
    }
}
pub(crate) fn get(db: &Connection, id: &str) -> Result<Value> {
    let row: Option<String> = db
        .query_row("SELECT document FROM tasks WHERE id=?1", [id], |r| r.get(0))
        .optional()
        .map_err(internal)?;
    serde_json::from_str(&row.ok_or_else(|| ApiError::new(404, "task_not_found"))?)
        .map_err(internal)
}
pub(crate) fn persist(tx: &Transaction<'_>, task: &Value, kind: &str) -> Result<()> {
    tx.execute("INSERT INTO tasks(id,document) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET document=excluded.document",params![task["id"].as_str(),task.to_string()]).map_err(internal)?;
    let event = json!({"type":kind,"task_id":task["id"],"task_revision":task["revision"],"at":task["updated_at"],"summary":kind.replace('.'," ")});
    tx.execute(
        "INSERT INTO events(task_id,document) VALUES(?1,?2)",
        params![task["id"].as_str(), event.to_string()],
    )
    .map_err(internal)?;
    Ok(())
}
fn text(value: &Value, max: usize, required: bool) -> bool {
    value.as_str().is_some_and(|v| {
        v.chars().count() <= max
            && (!required || !v.trim().is_empty())
            && !v
                .chars()
                .any(|c| c == '\0' || (c.is_control() && c != '\n' && c != '\r' && c != '\t'))
    })
}
fn nullable_text(value: &Value, max: usize) -> bool {
    value.is_null() || text(value, max, false)
}
fn repository(value: &str) -> bool {
    let Some((owner, repo)) = value.split_once('/') else {
        return false;
    };
    !owner.is_empty()
        && owner.len() <= 39
        && owner.as_bytes()[0].is_ascii_alphanumeric()
        && owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !repo.is_empty()
        && repo.len() <= 100
        && repo != "."
        && repo != ".."
        && !repo.ends_with(".git")
        && repo
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
pub(crate) fn validate(task: &Value, preserve_done: bool, repository_queue: bool) -> Result<()> {
    if !task["coding_agent_id"].is_null()
        && !task["coding_agent_id"].as_str().is_some_and(|s| {
            !s.is_empty()
                && s.len() <= 40
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_".contains(&b))
        })
    {
        return Err(ApiError::bad("coding_agent_unavailable"));
    }
    if !task["criticality"].is_null()
        && !["critical", "high", "medium", "low"]
            .contains(&task["criticality"].as_str().unwrap_or(""))
    {
        return Err(ApiError::bad("invalid_criticality"));
    }
    if !text(&task["title"], 240, true) || !text(&task["objective"], 8000, true) {
        return Err(ApiError::bad("invalid_task_text"));
    }
    for key in ["project", "owner"] {
        if !nullable_text(&task[key], 160) {
            return Err(ApiError::bad("invalid_task_text"));
        }
    }
    if !nullable_text(&task["blocked_reason"], 1000) {
        return Err(ApiError::bad("invalid_task_text"));
    }
    for key in ["acceptance_criteria", "context"] {
        if !task[key]
            .as_array()
            .is_some_and(|a| a.len() <= 50 && a.iter().all(|v| text(v, 2000, true)))
        {
            return Err(ApiError::bad("invalid_task_list"));
        }
    }
    if !task["priority"].as_u64().is_some_and(|n| n <= 4) {
        return Err(ApiError::bad("invalid_priority"));
    }
    if !["manual", "engineering"].contains(&task["workflow"].as_str().unwrap_or("")) {
        return Err(ApiError::bad("invalid_workflow"));
    }
    if !["backlog", "ready", "active", "review", "done", "cancelled"]
        .contains(&task["status"].as_str().unwrap_or(""))
    {
        return Err(ApiError::bad("invalid_task_status"));
    }
    if !["manual", "ready_to_done"].contains(&task["automation_policy"].as_str().unwrap_or("")) {
        return Err(ApiError::bad("automation_unavailable"));
    }
    if task["automation_policy"] == "ready_to_done"
        && (task["workflow"] != "engineering"
            || (!repository_queue && task["merge_policy"] != "auto"))
    {
        return Err(ApiError::bad("invalid_automation_policy"));
    }

    if !["required", "skip"].contains(&task["qa_policy"].as_str().unwrap_or(""))
        || !["manual", "auto"].contains(&task["merge_policy"].as_str().unwrap_or(""))
    {
        return Err(ApiError::bad("invalid_delivery_policy"));
    }
    if task["workflow"] == "engineering" {
        if !crate::research::eligible(task) && !task["repository"].as_str().is_some_and(repository)
        {
            return Err(ApiError::bad("invalid_repository"));
        }
        if ["active", "review"].contains(&task["status"].as_str().unwrap_or(""))
            || (task["status"] == "done" && !preserve_done)
        {
            return Err(ApiError::unavailable("execution_unavailable"));
        }
    } else if !task["repository"].is_null() {
        return Err(ApiError::bad("manual_repository_forbidden"));
    }
    for (key, kind) in [
        ("github_issue_url", "issues"),
        ("github_pull_request_url", "pull"),
    ] {
        if task[key].is_null() {
            continue;
        }
        let value = task[key]
            .as_str()
            .ok_or_else(|| ApiError::bad("invalid_github_url"))?;
        let parts: Vec<_> = value
            .strip_prefix("https://github.com/")
            .unwrap_or("")
            .split('/')
            .collect();
        if parts.len() != 4
            || parts[2] != kind
            || parts[3].is_empty()
            || parts[3].starts_with('0')
            || !parts[3].bytes().all(|b| b.is_ascii_digit())
            || !repository(&format!("{}/{}", parts[0], parts[1]))
            || (task["workflow"] == "engineering"
                && task["repository"].as_str() != Some(&format!("{}/{}", parts[0], parts[1])))
        {
            return Err(ApiError::bad("invalid_github_url"));
        }
    }
    if ["done", "cancelled"].contains(&task["status"].as_str().unwrap_or(""))
        && !task["blocked_reason"].is_null()
    {
        return Err(ApiError::conflict("task_blocked"));
    }
    Ok(())
}

pub(crate) fn valid_repository(value: &str) -> bool {
    repository(value)
}
