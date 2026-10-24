//! Human-editable agent content, separate from immutable host execution authority.
use crate::{ApiError, Result, execution_config::Policy, store::internal};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    pub id: String,
    pub label: String,
    pub role: String,
    pub instructions: String,
    pub model: String,
    pub enabled: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    pub main: String,
    pub coder: String,
    pub qa: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub revision: u64,
    pub agents: Vec<Agent>,
    pub defaults: Defaults,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assignment {
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_approval_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main: Option<Agent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coder: Option<Agent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qa: Option<Agent>,
}
impl Agent {
    fn validate(&self) -> Result<()> {
        if self.id.is_empty()
            || self.id.len() > 40
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_".contains(&b))
            || self.label.trim().is_empty()
            || self.label.len() > 80
            || !["main", "coder", "qa"].contains(&self.role.as_str())
            || self.instructions.len() > 2000
            || self.model.len() > 80
            || !self
                .model
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            || self.label.chars().any(char::is_control)
        {
            return Err(ApiError::bad("agent_registry_invalid"));
        }
        Ok(())
    }
    fn public(&self) -> Value {
        json!({"id":self.id,"label":self.label,"role":self.role,"model":self.model})
    }
}
impl Registry {
    pub fn validate(&self) -> Result<()> {
        if serde_json::to_vec(self).map_err(internal)?.len() > 8192 {
            return Err(ApiError::bad("agent_config_too_large"));
        }
        self.validate_shape()
    }
    fn validate_shape(&self) -> Result<()> {
        if self.agents.len() < 3
            || self.agents.len() > 16
            || serde_json::to_vec(self).map_err(internal)?.len() > 24576
        {
            return Err(ApiError::bad("agent_registry_invalid"));
        }
        let mut ids = std::collections::HashSet::new();
        for a in &self.agents {
            a.validate()?;
            if !ids.insert(&a.id) {
                return Err(ApiError::bad("agent_registry_invalid"));
            }
        }
        for (id, role) in [
            (&self.defaults.main, "main"),
            (&self.defaults.coder, "coder"),
            (&self.defaults.qa, "qa"),
        ] {
            self.find(id, role, false)?;
        }
        Ok(())
    }
    fn find(&self, id: &str, role: &str, enabled: bool) -> Result<&Agent> {
        self.agents
            .iter()
            .find(|a| a.id == id && a.role == role && (!enabled || a.enabled))
            .ok_or_else(|| ApiError::conflict("agent_unavailable"))
    }
    pub(crate) fn coder(&self, id: Option<&str>) -> Result<&Agent> {
        self.find(id.unwrap_or(&self.defaults.coder), "coder", true)
    }
    pub(crate) fn coding_agents(&self) -> Value {
        json!({"default_coder":self.defaults.coder,"agents":self.agents.iter().filter(|a|a.role=="coder"&&a.enabled).map(|a|json!({"id":a.id,"label":a.label})).collect::<Vec<_>>()})
    }
    pub(crate) fn assignment(&self, task: Option<&Value>) -> Result<Assignment> {
        Ok(if let Some(task) = task {
            Assignment {
                revision: self.revision,
                base_approval_sha256: None,
                main: None,
                coder: Some(self.coder(task["coding_agent_id"].as_str())?.clone()),
                qa: Some(
                    self.find(&self.defaults.qa, "qa", task["qa_policy"] != "skip")?
                        .clone(),
                ),
            }
        } else {
            Assignment {
                revision: self.revision,
                base_approval_sha256: None,
                main: Some(self.find(&self.defaults.main, "main", true)?.clone()),
                coder: None,
                qa: None,
            }
        })
    }
    pub(crate) fn public(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap();
        v["editable"] = json!(true);
        v
    }
}
impl Assignment {
    pub(crate) fn validate(&self) -> Result<()> {
        for (a, role) in [
            (&self.main, "main"),
            (&self.coder, "coder"),
            (&self.qa, "qa"),
        ] {
            if let Some(a) = a {
                a.validate()?;
                if a.role != role {
                    return Err(ApiError::bad("agent_registry_invalid"));
                }
            }
        }
        Ok(())
    }
    pub(crate) fn agent(&self, action: &str) -> Option<&Agent> {
        match action {
            "plan" => self.main.as_ref(),
            "coder" => self.coder.as_ref(),
            "qa" => self.qa.as_ref(),
            _ => None,
        }
    }
    pub(crate) fn public(&self) -> Value {
        json!({"revision":self.revision,"main":self.main.as_ref().map(Agent::public),"coder":self.coder.as_ref().map(Agent::public),"qa":self.qa.as_ref().map(Agent::public)})
    }
    pub(crate) fn apply(&self, policy: &mut Policy) -> Result<()> {
        let adapter = policy
            .adapter_policy
            .as_mut()
            .ok_or_else(|| ApiError::unavailable("execution_unavailable"))?;
        adapter.agent_config = Some(self.clone());
        if self.main.is_some() {
            if let Some(native) = &mut adapter.native_agent {
                for agent in &mut native.agents {
                    agent.instructions.clear();
                }
            }
        }
        if let (Some(native), Some(coder), Some(qa)) =
            (&mut adapter.native_agent, &self.coder, &self.qa)
        {
            native.default_coder = coder.id.clone();
            native.qa_agent = qa.id.clone();
            native.agents = vec![coder, qa]
                .into_iter()
                .map(|a| crate::adapters::native::Agent {
                    id: a.id.clone(),
                    label: a.label.clone(),
                    instructions: String::new(),
                })
                .collect();
        }
        Ok(())
    }
}
pub(crate) fn initialize(
    db: &Connection,
    policy: Option<&crate::adapters::native::NativeAgentPolicy>,
) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS system_agents(singleton INTEGER PRIMARY KEY CHECK(singleton=1),document TEXT NOT NULL)").map_err(internal)?;
    let exists: bool = db
        .query_row("SELECT EXISTS(SELECT 1 FROM system_agents)", [], |r| {
            r.get(0)
        })
        .map_err(internal)?;
    if exists {
        return Ok(());
    }
    let mut agents = vec![];
    if let Some(p) = policy {
        agents.extend(p.agents.iter().map(|a| Agent {
            id: a.id.clone(),
            label: a.label.clone(),
            role: if a.id == p.qa_agent { "qa" } else { "coder" }.into(),
            instructions: a.instructions.clone(),
            model: String::new(),
            enabled: true,
        }));
    }
    let default_coder = policy
        .map(|p| p.default_coder.clone())
        .unwrap_or_else(|| "coder".into());
    let default_qa = policy
        .map(|p| p.qa_agent.clone())
        .unwrap_or_else(|| "qa".into());
    if policy.is_none() {
        for (id, label, role, instructions) in [
            (
                "coder",
                "Coder",
                "coder",
                "Implement the task, run relevant checks and inspect the whole diff.",
            ),
            (
                "qa",
                "QA",
                "qa",
                "Independently validate the exact implementation and related regressions.",
            ),
        ] {
            agents.push(Agent {
                id: id.into(),
                label: label.into(),
                role: role.into(),
                instructions: instructions.into(),
                model: String::new(),
                enabled: true,
            });
        }
    }
    let main = if agents.iter().any(|a| a.id == "main") {
        "workboard-main"
    } else {
        "main"
    }
    .to_string();
    agents.insert(0,Agent{id:main.clone(),label:"Main".into(),role:"main".into(),instructions:"Propose small coherent tasks for the supplied goal; preserve constraints and expose assumptions.".into(),model:String::new(),enabled:true});
    let value = Registry {
        revision: 1,
        agents,
        defaults: Defaults {
            main,
            coder: default_coder,
            qa: default_qa,
        },
    };
    value.validate_shape()?;
    db.execute(
        "INSERT OR IGNORE INTO system_agents VALUES(1,?1)",
        [serde_json::to_string(&value).map_err(internal)?],
    )
    .map_err(internal)?;
    Ok(())
}
pub(crate) fn get(db: &Connection) -> Result<Registry> {
    let raw: String = db
        .query_row(
            "SELECT document FROM system_agents WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(internal)?;
    serde_json::from_str(&raw).map_err(internal)
}
impl crate::store::Store {
    pub(crate) fn system_command(
        &mut self,
        key: &str,
        fingerprint: &str,
        body: Value,
        factory: Option<&crate::projects::Factory>,
    ) -> Result<Value> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(internal)?;
        if let Some((prior, response)) = tx
            .query_row(
                "SELECT fingerprint,response FROM commands WHERE key=?1",
                [key],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(internal)?
        {
            if prior != fingerprint {
                return Err(ApiError::conflict("idempotency_conflict"));
            }
            return serde_json::from_str(&response).map_err(internal);
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Update {
            expected_revision: u64,
            agents: Vec<Agent>,
            defaults: Defaults,
        }
        let input: Update =
            serde_json::from_value(body).map_err(|_| ApiError::bad("agent_registry_invalid"))?;
        let current = get(&tx)?;
        if current.revision != input.expected_revision {
            return Err(ApiError::conflict("revision_conflict"));
        }
        let next = Registry {
            revision: current.revision + 1,
            agents: input.agents,
            defaults: input.defaults,
        };
        next.validate()?;
        if let Some(factory) = factory {
            factory.validate_registry(&next)?;
        }
        tx.execute(
            "UPDATE system_agents SET document=?1 WHERE singleton=1",
            [serde_json::to_string(&next).map_err(internal)?],
        )
        .map_err(internal)?;
        tx.execute("INSERT INTO events(task_id,document) VALUES(NULL,?1)",[json!({"type":"system.updated","at":crate::store::now(),"summary":"Agent settings updated"}).to_string()]).map_err(internal)?;
        let response = json!({"system":next.public()});
        tx.execute(
            "INSERT INTO commands VALUES(?1,?2,200,?3)",
            params![key, fingerprint, response.to_string()],
        )
        .map_err(internal)?;
        tx.commit().map_err(internal)?;
        Ok(response)
    }
}
