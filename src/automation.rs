//! Single-use, separately signed automatic admission. No task-selected commands.
use crate::{
    ApiError, Result,
    execution_config::{Approval, now_ms},
};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::Value;
use sha2::Sha256;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    schema_version: u32,
    repository: String,
    execution_approval_sha256: String,
    ownership_record_id: String,
    ownership_epoch: u64,
    expires_at_ms: i64,
    max_runs: u8,
}
pub struct Activation {
    policy: Policy,
    pub(crate) digest: String,
    path: PathBuf,
    key: Arc<Vec<u8>>,
}
impl Activation {
    pub fn from_env() -> Option<Self> {
        let path = PathBuf::from(std::env::var_os("WORKBOARD_AUTOMATION_CONFIG")?);
        let key = std::env::var("WORKBOARD_AUTOMATION_VERIFY_KEY").ok()?;
        let digest = std::env::var("WORKBOARD_AUTOMATION_APPROVAL_SHA256").ok()?;
        Self::load(&path, key.as_bytes(), &digest).ok()
    }
    pub fn load(path: &Path, key: &[u8], digest: &str) -> Result<Self> {
        if !(32..=256).contains(&key.len()) || !crate::execution_config::hash(digest) {
            return Err(ApiError::bad("automation_activation_invalid"));
        }
        crate::execution_config::private_file(path)?;
        let bytes = std::fs::read(path).map_err(|_| ApiError::internal())?;
        if crate::ownership::sha(&bytes) != digest {
            return Err(ApiError::bad("automation_activation_invalid"));
        }
        let envelope: Value = serde_json::from_slice(&bytes)
            .map_err(|_| ApiError::bad("automation_activation_invalid"))?;
        if envelope.as_object().is_none_or(|o| {
            o.len() != 2 || !o.contains_key("payload") || !o.contains_key("signature")
        }) {
            return Err(ApiError::bad("automation_activation_invalid"));
        }
        let signature = envelope["signature"]
            .as_str()
            .and_then(crate::execution_config::decode)
            .ok_or_else(|| ApiError::bad("automation_activation_invalid"))?;
        let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(|_| ApiError::internal())?;
        mac.update(envelope["payload"].to_string().as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| ApiError::bad("automation_activation_invalid"))?;
        let policy: Policy = serde_json::from_value(envelope["payload"].clone())
            .map_err(|_| ApiError::bad("automation_activation_invalid"))?;
        if policy.schema_version != 1
            || policy.max_runs != 1
            || !crate::store::valid_repository(&policy.repository)
            || !crate::execution_config::hash(&policy.execution_approval_sha256)
            || uuid::Uuid::parse_str(&policy.ownership_record_id).is_err()
            || policy.ownership_epoch == 0
        {
            return Err(ApiError::bad("automation_activation_invalid"));
        }
        Ok(Self {
            policy,
            digest: digest.into(),
            path: path.into(),
            key: Arc::new(key.to_vec()),
        })
    }
    pub(crate) fn verify(&self, approval: &Approval) -> Result<()> {
        let current = Self::load(&self.path, &self.key, &self.digest)?;
        let owner = approval
            .policy
            .ownership
            .as_ref()
            .ok_or_else(|| ApiError::unavailable("automation_unavailable"))?;
        if current.policy.expires_at_ms <= now_ms()
            || current.policy.expires_at_ms > approval.policy.expires_at_ms
            || current.policy.execution_approval_sha256 != approval.digest
            || current.policy.ownership_record_id != owner.record_id
            || current.policy.ownership_epoch != owner.epoch
            || current.policy.repository != owner.repository
        {
            return Err(ApiError::unavailable("automation_unavailable"));
        }
        approval.verify(&approval.digest, false)
    }
    pub(crate) fn repository(&self) -> &str {
        &self.policy.repository
    }
    pub(crate) fn eligible(&self, task: &Value) -> bool {
        task["repository"] == self.policy.repository
            && task["workflow"] == "engineering"
            && task["status"] == "ready"
            && ["required", "skip"].contains(&task["qa_policy"].as_str().unwrap_or("required"))
            && task["merge_policy"] == "auto"
            && task["automation_policy"] == "ready_to_done"
            && task["automation_revision"] == task["revision"]
            && task["blocked_reason"].is_null()
            && task["archived_at"].is_null()
    }
}
