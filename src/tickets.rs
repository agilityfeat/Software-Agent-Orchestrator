//! Ticket metadata is descriptive; only explicit dependency and QA fields gate execution.
use crate::{ApiError, Result, store::internal};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};
use std::collections::HashSet;
use uuid::Uuid;
pub const TYPES: [&str; 5] = ["feature", "bug", "improvement", "chore", "research"];
pub(crate) fn validate_size(task: &Value) -> Result<()> {
    if serde_json::to_vec(task).map_err(internal)?.len() > 32768 {
        return Err(ApiError::bad("task_too_large"));
    }
    Ok(())
}
pub fn feature() -> String {
    "feature".into()
}
pub fn labels(values: &[String]) -> Result<Vec<String>> {
    if values.len() > 12 {
        return Err(ApiError::bad("invalid_labels"));
    }
    let mut out = Vec::new();
    for value in values {
        let value = value
            .trim()
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("-");
        if value.is_empty()
            || value.chars().count() > 32
            || !value
                .chars()
                .all(|c| c.is_alphanumeric() || "-_. /".contains(c))
        {
            return Err(ApiError::bad("invalid_labels"));
        }
        if !out.contains(&value) {
            out.push(value);
        }
    }
    Ok(out)
}
pub fn normalize(task: &mut Value) -> Result<()> {
    if task.get("qa_policy").is_none() {
        task["qa_policy"] = json!("required");
    }
    if task.get("task_type").is_none() {
        task["task_type"] = json!("feature");
    }
    if !TYPES.contains(&task["task_type"].as_str().unwrap_or("")) {
        return Err(ApiError::bad("invalid_task_type"));
    }
    if task.get("labels").is_none() {
        task["labels"] = json!([]);
    }
    let raw: Vec<String> = serde_json::from_value(task["labels"].clone())
        .map_err(|_| ApiError::bad("invalid_labels"))?;
    task["labels"] = json!(labels(&raw)?);
    if task.get("dependencies").is_none() {
        task["dependencies"] = json!([]);
    }
    let deps: Vec<String> = serde_json::from_value(task["dependencies"].clone())
        .map_err(|_| ApiError::bad("invalid_dependencies"))?;
    let mut seen = HashSet::new();
    if deps.len() > 16
        || deps
            .iter()
            .any(|d| Uuid::parse_str(d).is_err() || !seen.insert(d))
    {
        return Err(ApiError::bad("invalid_dependencies"));
    }
    Ok(())
}
fn document(db: &Connection, id: &str) -> Result<Value> {
    let raw: Option<String> = db
        .query_row("SELECT document FROM tasks WHERE id=?1", [id], |r| r.get(0))
        .optional()
        .map_err(internal)?;
    serde_json::from_str(&raw.ok_or_else(|| ApiError::bad("dependency_not_found"))?)
        .map_err(internal)
}
pub fn validate_dependencies(db: &Connection, task: &Value) -> Result<()> {
    let own = task["id"].as_str().unwrap_or("");
    let mut pending: Vec<String> = task["dependencies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect();
    let mut seen = HashSet::new();
    while let Some(id) = pending.pop() {
        if id == own {
            return Err(ApiError::bad("dependency_cycle"));
        }
        if !seen.insert(id.clone()) {
            continue;
        }
        let next = document(db, &id)?;
        pending.extend(
            next["dependencies"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(String::from),
        );
    }
    Ok(())
}
pub fn blockers(db: &Connection, task: &Value) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for id in task["dependencies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let dep = document(db, id)?;
        if dep["status"] != "done" {
            out.push(json!({"id":id,"title":dep["title"],"status":dep["status"]}));
        }
    }
    Ok(out)
}
pub fn project(db: &Connection, mut task: Value) -> Result<Value> {
    for (key, value) in [
        ("task_type", json!("feature")),
        ("labels", json!([])),
        ("dependencies", json!([])),
        ("qa_policy", json!("required")),
    ] {
        if task.get(key).is_none() {
            task[key] = value;
        }
    }
    task["dependency_blockers"] = json!(blockers(db, &task)?);
    Ok(task)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_projection_defaults_without_modifying_the_document() {
        let db = Connection::open_in_memory().unwrap();
        let raw = json!({"id":"old","title":"Legacy"});
        let projected = project(&db, raw.clone()).unwrap();
        assert_eq!(projected["task_type"], "feature");
        assert_eq!(projected["qa_policy"], "required");
        assert_eq!(projected["labels"], json!([]));
        assert_eq!(projected["dependencies"], json!([]));
        assert!(raw.get("task_type").is_none());
    }
    #[test]
    fn only_done_satisfies_dependencies_even_when_archived() {
        let db = Connection::open_in_memory().unwrap();
        db.execute("CREATE TABLE tasks(id TEXT PRIMARY KEY,document TEXT)", [])
            .unwrap();
        let id = Uuid::new_v4().to_string();
        let task = json!({"dependencies":[id]});
        for status in ["ready", "review", "cancelled", "done"] {
            db.execute(
                "INSERT OR REPLACE INTO tasks VALUES(?1,?2)",
                rusqlite::params![
                    id,
                    json!({"title":"Prerequisite","status":status,"archived_at":"retained"})
                        .to_string()
                ],
            )
            .unwrap();
            assert_eq!(blockers(&db, &task).unwrap().is_empty(), status == "done");
        }
    }
}
