use crate::{ApiError, Result};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Program {
    pub executable: PathBuf,
    pub sha256: String,
    #[serde(default)]
    pub credential_env_names: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub schema_version: u32,
    pub containment_contract: String,
    pub expires_at_ms: i64,
    pub work_root: PathBuf,
    pub worker: Program,
    pub publisher: Program,
    pub git: Program,
    pub max_run_seconds: u64,
    pub max_stage_seconds: u64,
    pub max_output_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_qa_repairs: Option<u8>,
    #[serde(default)]
    pub adapter_policy: Option<crate::adapters::AdapterPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ownership: Option<crate::ownership::Binding>,
}
#[derive(Clone)]
pub struct Approval {
    pub(crate) policy: Policy,
    pub(crate) digest: String,
    path: PathBuf,
    key: Arc<Vec<u8>>,
}
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
pub fn digest_file(path: &Path) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(std::fs::read(path).map_err(|_| ApiError::bad("execution_pin_invalid"))?)
    ))
}
pub(crate) fn private_file(path: &Path) -> Result<()> {
    let meta =
        std::fs::symlink_metadata(path).map_err(|_| ApiError::bad("execution_config_invalid"))?;
    if !path.is_absolute() || !meta.is_file() || meta.len() > 16384 {
        return Err(ApiError::bad("execution_config_invalid"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(ApiError::bad("execution_config_invalid"));
        }
    }
    Ok(())
}
impl Approval {
    pub fn from_env() -> Option<Self> {
        let path = PathBuf::from(std::env::var_os("WORKBOARD_EXECUTION_CONFIG")?);
        let key = std::env::var("WORKBOARD_EXECUTION_VERIFY_KEY").ok()?;
        let digest = std::env::var("WORKBOARD_EXECUTION_APPROVAL_SHA256").ok()?;
        Self::load(&path, key.as_bytes(), &digest).ok()
    }
    pub fn load(path: &Path, key: &[u8], digest: &str) -> Result<Self> {
        if key.len() < 32 || key.len() > 256 || !hash(digest) {
            return Err(ApiError::bad("execution_approval_invalid"));
        }
        private_file(path)?;
        let bytes = std::fs::read(path).map_err(|_| ApiError::bad("execution_config_invalid"))?;
        if format!("{:x}", Sha256::digest(&bytes)) != digest {
            return Err(ApiError::bad("execution_approval_invalid"));
        }
        let envelope: Value = serde_json::from_slice(&bytes)
            .map_err(|_| ApiError::bad("execution_config_invalid"))?;
        if envelope.as_object().is_none_or(|o| {
            o.len() != 2 || !o.contains_key("payload") || !o.contains_key("signature")
        }) {
            return Err(ApiError::bad("execution_config_invalid"));
        }
        let signature = envelope["signature"]
            .as_str()
            .and_then(decode)
            .ok_or_else(|| ApiError::bad("execution_approval_invalid"))?;
        let mut mac = Hmac::<Sha256>::new_from_slice(key)
            .map_err(|_| ApiError::bad("execution_approval_invalid"))?;
        mac.update(envelope["payload"].to_string().as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| ApiError::bad("execution_approval_invalid"))?;
        let policy: Policy = serde_json::from_value(envelope["payload"].clone())
            .map_err(|_| ApiError::bad("execution_config_invalid"))?;
        if policy.schema_version != 1
            || policy.containment_contract != "reviewed-launcher-v1"
            || !(1..=14400).contains(&policy.max_run_seconds)
            || !(1..=3600).contains(&policy.max_stage_seconds)
            || policy.max_qa_repairs.is_some_and(|n| n > 2)
            || !(1024..=1048576).contains(&policy.max_output_bytes)
        {
            return Err(ApiError::bad("execution_config_invalid"));
        }
        if let Some(adapter) = &policy.adapter_policy {
            adapter.validate()?;
        }
        check_envs(
            &policy.worker,
            &["CODEX_HOME", "OPENAI_API_KEY", "CODEX_API_KEY"],
        )?;
        check_envs(&policy.publisher, &["GH_TOKEN", "GITHUB_TOKEN"])?;
        check_envs(&policy.git, &[])?;
        let approval = Self {
            policy,
            digest: digest.into(),
            path: path.into(),
            key: Arc::new(key.to_vec()),
        };
        approval.verify(&approval.digest, true)?;
        Ok(approval)
    }
    pub(crate) fn with_agents(&self, mut agents: crate::system::Assignment) -> Result<Self> {
        self.verify(&self.digest, false)?;
        agents.base_approval_sha256 = Some(self.digest.clone());
        let mut policy = self.policy.clone();
        agents.apply(&mut policy)?;
        let payload = serde_json::to_value(policy).map_err(|_| ApiError::internal())?;
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.key).map_err(|_| ApiError::internal())?;
        mac.update(payload.to_string().as_bytes());
        let envelope = serde_json::json!({"payload":payload,"signature":format!("{:x}",mac.finalize().into_bytes())});
        let path = self
            .path
            .with_file_name(format!("agents-{}.json", uuid::Uuid::new_v4()));
        use std::io::Write;
        let mut file = crate::storage_ownership::private_options()
            .create_new(true)
            .open(&path)
            .map_err(|_| ApiError::internal())?;
        file.write_all(envelope.to_string().as_bytes())
            .map_err(|_| ApiError::internal())?;
        file.sync_all().map_err(|_| ApiError::internal())?;
        Self::load(&path, &self.key, &digest_file(&path)?)
    }
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
    pub fn verify(&self, run_digest: &str, observation: bool) -> Result<()> {
        if self.digest != run_digest {
            return Err(ApiError::bad("execution_approval_changed"));
        }
        private_file(&self.path)?;
        if digest_file(&self.path)? != self.digest {
            return Err(ApiError::bad("execution_approval_changed"));
        }
        // The signed bytes and approval digest are immutable; every launch rechecks the exact same envelope.
        if self.key.len() < 32 {
            return Err(ApiError::bad("execution_approval_invalid"));
        }
        if !observation && self.policy.expires_at_ms <= now_ms() {
            return Err(ApiError::bad("execution_approval_expired"));
        }
        let root = &self.policy.work_root;
        let meta =
            std::fs::symlink_metadata(root).map_err(|_| ApiError::bad("execution_root_invalid"))?;
        if !root.is_absolute() || !meta.is_dir() || root.canonicalize().ok().as_ref() != Some(root)
        {
            return Err(ApiError::bad("execution_root_invalid"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o077 != 0 {
                return Err(ApiError::bad("execution_root_invalid"));
            }
        }
        for program in [
            &self.policy.worker,
            &self.policy.publisher,
            &self.policy.git,
        ] {
            let meta = std::fs::symlink_metadata(&program.executable)
                .map_err(|_| ApiError::bad("execution_pin_invalid"))?;
            if !program.executable.is_absolute()
                || !meta.is_file()
                || !hash(&program.sha256)
                || digest_file(&program.executable)? != program.sha256
            {
                return Err(ApiError::bad("execution_pin_invalid"));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if meta.permissions().mode() & 0o022 != 0 || meta.permissions().mode() & 0o111 == 0
                {
                    return Err(ApiError::bad("execution_pin_invalid"));
                }
            }
        }
        Ok(())
    }
}
fn check_envs(program: &Program, allowed: &[&str]) -> Result<()> {
    let mut unique = HashSet::new();
    if program.credential_env_names.len() > allowed.len()
        || program
            .credential_env_names
            .iter()
            .any(|name| !allowed.contains(&name.as_str()) || !unique.insert(name))
    {
        return Err(ApiError::bad("execution_credential_scope_invalid"));
    }
    Ok(())
}
pub fn sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
pub(crate) fn decode(value: &str) -> Option<Vec<u8>> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return None;
    }
    (0..64)
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).ok())
        .collect()
}

pub(crate) fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
