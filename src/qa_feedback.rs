//! Bounded actionable review data; never commands or authority to expand scope.
use crate::{ApiError, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub path: String,
    pub line: u32,
    pub summary: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repair {
    pub cycle: u8,
    pub reviewed_sha: String,
    pub qa_action_id: String,
    pub qa_session_id: String,
    pub findings: Vec<Finding>,
}
pub fn findings(values: &[Finding]) -> Result<()> {
    if values.len() > 8 {
        return Err(ApiError::bad("qa_findings_invalid"));
    }
    let mut total = 0;
    for f in values {
        total += f.path.len() + f.summary.len();
        if f.path.is_empty()
            || f.path.len() > 240
            || f.path.starts_with('/')
            || f.path.contains(['\\', ':'])
            || f.path.chars().any(char::is_control)
            || f.path
                .split('/')
                .any(|s| s.is_empty() || s == "." || s == "..")
            || f.line == 0
            || f.line > 1000000
            || f.summary.trim().len() < 12
            || f.summary.len() > 1000
            || f.summary
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err(ApiError::bad("qa_findings_invalid"));
        }
    }
    if total > 4096 {
        return Err(ApiError::bad("qa_findings_invalid"));
    }
    Ok(())
}
impl Repair {
    pub fn validate(&self, head: &str) -> Result<()> {
        if !(1..=2).contains(&self.cycle)
            || self.reviewed_sha != head
            || !crate::execution_config::sha(head)
            || uuid::Uuid::parse_str(&self.qa_action_id).is_err()
            || !(8..=128).contains(&self.qa_session_id.len())
            || !self
                .qa_session_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-:.".contains(&b))
            || self.findings.is_empty()
        {
            return Err(ApiError::bad("qa_repair_context_invalid"));
        }
        findings(&self.findings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn feedback_is_finite_relative_actionable_data_and_allows_missing_files() {
        let f = Finding {
            path: "src/missing.rs".into(),
            line: 1,
            summary: "Implement the missing required error handling branch.".into(),
        };
        assert!(findings(std::slice::from_ref(&f)).is_ok());
        for path in [
            "../outside",
            "/absolute",
            "src/../outside",
            "src\\file",
            "file:\\secret",
            "",
        ] {
            let mut bad = f.clone();
            bad.path = path.into();
            assert!(findings(&[bad]).is_err());
        }
        for text in ["", "unclear", &"x".repeat(1001)] {
            let mut bad = f.clone();
            bad.summary = text.into();
            assert!(findings(&[bad]).is_err());
        }
        assert!(findings(&vec![f.clone(); 9]).is_err());
        let mut long = f;
        long.summary = "x".repeat(999);
        assert!(findings(&vec![long; 5]).is_err());
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Criticality {
    Critical,
    High,
    Medium,
    Low,
}
impl Criticality {
    pub fn priority(&self) -> u8 {
        match self {
            Self::Critical => 1,
            Self::High => 2,
            Self::Medium => 3,
            Self::Low => 4,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutOfScopeBug {
    pub path: String,
    pub line: u32,
    pub summary: String,
    pub criticality: Criticality,
    pub reproduction: String,
    pub expected: String,
    pub actual: String,
    pub base_sha: String,
    pub base_reproduction: String,
    pub unrelated_reason: String,
}
pub fn out_of_scope_bugs(values: &[OutOfScopeBug], base: &str) -> Result<()> {
    if values.len() > 4 {
        return Err(ApiError::bad("qa_out_of_scope_invalid"));
    }
    let mut total = 0;
    for bug in values {
        findings(&[Finding {
            path: bug.path.clone(),
            line: bug.line,
            summary: bug.summary.clone(),
        }])
        .map_err(|_| ApiError::bad("qa_out_of_scope_invalid"))?;
        if bug.summary.len() > 240 || bug.base_sha != base || !crate::execution_config::sha(base) {
            return Err(ApiError::bad("qa_out_of_scope_invalid"));
        }
        total += bug.path.len() + bug.summary.len();
        for (text, max) in [
            (&bug.reproduction, 1000),
            (&bug.expected, 500),
            (&bug.actual, 500),
            (&bug.base_reproduction, 1000),
            (&bug.unrelated_reason, 1000),
        ] {
            if text.trim().len() < 12
                || text.len() > max
                || text
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t')
            {
                return Err(ApiError::bad("qa_out_of_scope_invalid"));
            }
            total += text.len();
        }
    }
    if total > 8192 {
        return Err(ApiError::bad("qa_out_of_scope_invalid"));
    }
    Ok(())
}
pub fn result_schema() -> serde_json::Value {
    use serde_json::json;
    let location =
        json!({"path":{"type":"string"},"line":{"type":"integer"},"summary":{"type":"string"}});
    let mut properties = location.clone();
    let p = properties.as_object_mut().unwrap();
    for name in [
        "reproduction",
        "expected",
        "actual",
        "base_sha",
        "base_reproduction",
        "unrelated_reason",
    ] {
        p.insert(name.into(), json!({"type":"string"}));
    }
    p.insert(
        "criticality".into(),
        json!({"type":"string","enum":["critical","high","medium","low"]}),
    );
    json!({"type":"object","additionalProperties":false,"required":["completed","passed","findings","out_of_scope_bugs"],"properties":{"completed":{"type":"boolean"},"passed":{"type":"boolean"},"findings":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["path","line","summary"],"properties":location}},"out_of_scope_bugs":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["path","line","summary","criticality","reproduction","expected","actual","base_sha","base_reproduction","unrelated_reason"],"properties":properties}}}})
}

#[cfg(test)]
mod bug_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn unrelated_bugs_require_bounded_baseline_proof_and_no_task_authority() {
        let value = json!({"path":"old/missing.rs","line":2,"summary":"A separate legacy error remains reproducible.","criticality":"critical","reproduction":"Call the old boundary with the documented input.","expected":"Return the documented legacy value.","actual":"Returns the same old invalid value.","base_sha":"a".repeat(40),"base_reproduction":"The exact base exhibits the same unrelated error.","unrelated_reason":"This separate path is not changed, exposed or worsened by this task."});
        let good: OutOfScopeBug = serde_json::from_value(value.clone()).unwrap();
        assert!(out_of_scope_bugs(std::slice::from_ref(&good), &"a".repeat(40)).is_ok());
        assert_eq!(good.criticality.priority(), 1);
        for key in ["repository", "status", "owner", "automation_policy"] {
            let mut bad = value.clone();
            bad[key] = json!("foreign/authority");
            assert!(serde_json::from_value::<OutOfScopeBug>(bad).is_err());
        }
        for (key, v) in [
            ("criticality", json!("urgent")),
            ("base_reproduction", json!("")),
            ("unrelated_reason", json!("unsure")),
            ("path", json!("../outside")),
            ("base_sha", json!("b".repeat(40))),
            ("reproduction", json!("x".repeat(1001))),
        ] {
            let mut bad = value.clone();
            bad[key] = v;
            assert!(match serde_json::from_value::<OutOfScopeBug>(bad) {
                Err(_) => true,
                Ok(b) => out_of_scope_bugs(&[b], &"a".repeat(40)).is_err(),
            });
        }
        assert!(out_of_scope_bugs(&vec![good; 5], &"a".repeat(40)).is_err());
        for (name, n) in [("critical", 1), ("high", 2), ("medium", 3), ("low", 4)] {
            let c: Criticality = serde_json::from_value(json!(name)).unwrap();
            assert_eq!(c.priority(), n);
        }
    }
}
