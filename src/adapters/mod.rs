//! Fixed real-pilot adapters. Task data never selects a host program or container profile.
pub mod contained;
mod profiles;
mod publisher;
pub use profiles::ValidationProfile;
pub(crate) use profiles::{validate_unit_source, validate_vite_inputs};
pub mod native;
mod worker;
use crate::{
    ApiError, Result,
    execution::Receipt,
    execution_config::{Program, digest_file, now_ms},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterPolicy {
    pub repository: String,
    pub state_root: PathBuf,
    pub auth_home: PathBuf,
    pub docker: Program,
    pub git: Program,
    pub gh: Program,
    pub docker_host: String,
    pub worker_image: String,
    pub worker_network: String,
    pub proxy_container: String,
    pub proxy_image: String,
    pub proxy_ip: String,
    pub proxy_port: u16,
    pub proxy_policy_sha256: String,
    pub proxy_allowed_hosts: Vec<String>,
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_profile: Option<ValidationProfile>,
    pub cpus: u8,
    pub memory_mb: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_agent: Option<native::NativeAgentPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_config: Option<crate::system::Assignment>,
}
impl AdapterPolicy {
    pub fn validate(&self) -> Result<()> {
        if let Some(config) = &self.agent_config {
            config.validate()?;
        }
        if let Some(native) = &self.native_agent {
            if !crate::store::valid_repository(&self.repository)
                || self.validation_profile.is_some()
            {
                return Err(err("adapter_policy_invalid"));
            }
            private_dir(&self.state_root)?;
            private_dir(&self.auth_home)?;
            pin(&self.git)?;
            pin(&self.gh)?;
            return native.validate();
        }
        if !crate::store::valid_repository(&self.repository)
            || !self.state_root.is_absolute()
            || !self.auth_home.is_absolute()
            || !self.docker_host.starts_with("unix:///")
            || self.docker_host.contains(['\n', '\r'])
            || !image(&self.worker_image)
            || !image(&self.proxy_image)
            || !hex64(&self.worker_network)
            || !hex64(&self.proxy_container)
            || !hex64(&self.proxy_policy_sha256)
            || self.proxy_ip.parse::<std::net::Ipv4Addr>().is_err()
            || self.proxy_port != 3128
            || !(1..=4).contains(&self.cpus)
            || !(256..=4096).contains(&self.memory_mb)
            || self.proxy_allowed_hosts.is_empty()
            || self.proxy_allowed_hosts.len() > 8
            || self.proxy_allowed_hosts.iter().any(|h| {
                !["api.openai.com", "chatgpt.com", "auth.openai.com"].contains(&h.as_str())
            })
            || self.proxy_allowed_hosts.windows(2).any(|w| w[0] >= w[1])
            || self.model.as_ref().is_some_and(|m| {
                m.is_empty()
                    || m.len() > 80
                    || !m
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            })
        {
            return Err(err("adapter_policy_invalid"));
        }
        if let Some(profile) = &self.validation_profile {
            profile.validate()?;
        }
        for program in [&self.docker, &self.git, &self.gh] {
            pin(program)?;
            if !program.credential_env_names.is_empty() {
                return Err(err("adapter_policy_invalid"));
            }
        }
        private_dir(&self.state_root)?;
        Ok(())
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema_version: u32,
    pub action: String,
    pub action_id: String,
    pub run_id: String,
    pub repository: String,
    pub task_snapshot: Value,
    pub deadline_ms: i64,
    pub stage_deadline_ms: i64,
    pub workspace: PathBuf,
    pub base_sha: Option<String>,
    pub head_sha: Option<String>,
    pub base_ref: Option<String>,
    pub qa_receipt: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coder_receipt: Option<Value>,
    pub merge_policy: String,
    pub observed_action_id: Option<String>,
    pub approval_sha256: String,
    pub adapter_policy: AdapterPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair_context: Option<crate::qa_feedback::Repair>,
}
impl Request {
    fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || Uuid::parse_str(&self.run_id).is_err()
            || Uuid::parse_str(&self.action_id).is_err()
            || self.repository != self.adapter_policy.repository
            || self.task_snapshot["repository"] != self.repository
            || self.task_snapshot["workflow"] != "engineering"
            || self.task_snapshot["merge_policy"] != self.merge_policy
            || !["manual", "auto"].contains(&self.merge_policy.as_str())
            || !hex64(&self.approval_sha256)
            || !self.workspace.is_absolute()
            || self.workspace.file_name().and_then(|v| v.to_str()) != Some(&self.run_id)
        {
            return Err(err("adapter_request_invalid"));
        }
        if self.action == "plan"
            && (self.adapter_policy.validation_profile.is_some()
                || self.base_sha.is_some()
                || self.head_sha.is_some()
                || self.repair_context.is_some())
        {
            return Err(err("adapter_request_invalid"));
        }
        if let Some(repair) = &self.repair_context {
            if self.action != "coder" {
                return Err(err("adapter_request_invalid"));
            }
            repair.validate(self.head_sha.as_deref().unwrap_or(""))?;
        }
        if ["plan", "coder", "qa"].contains(&self.action.as_str()) {
            if let Some(config) = &self.adapter_policy.agent_config {
                if !config.agent(&self.action).is_some_and(|a| a.enabled) {
                    return Err(err("agent_unavailable"));
                }
            }
        }
        self.adapter_policy.validate()
    }
    fn receipt(&self) -> Receipt {
        Receipt {
            schema_version: 1,
            action_id: self.action_id.clone(),
            run_id: self.run_id.clone(),
            action: self.action.clone(),
            repository: self.repository.clone(),
            outcome: "ok".into(),
            cleanup_complete: true,
            workspace: Some(self.workspace.to_string_lossy().into()),
            base_sha: self.base_sha.clone(),
            head_sha: self.head_sha.clone(),
            base_ref: self.base_ref.clone(),
            session_id: None,
            role: None,
            reviewed_sha: None,
            passed: None,
            pull_request_url: None,
            merged: None,
            merge_commit_sha: None,
            observed_action_id: self.observed_action_id.clone(),
            findings: None,
            qa_completed: None,
            out_of_scope_bugs: None,
            plan: None,
        }
    }
    fn scope(&self) -> Value {
        json!({"run_id":self.run_id,"repository":self.repository,"workspace":self.workspace,"task_snapshot":self.task_snapshot,"merge_policy":self.merge_policy,"approval_sha256":self.approval_sha256,"deadline_ms":self.deadline_ms})
    }
}
pub fn err(code: &'static str) -> ApiError {
    ApiError::unavailable(code)
}
fn hex64(v: &str) -> bool {
    v.len() == 64
        && v.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn image(v: &str) -> bool {
    v.strip_prefix("sha256:").is_some_and(hex64)
}
pub(crate) fn pin(p: &Program) -> Result<()> {
    let m = std::fs::symlink_metadata(&p.executable).map_err(|_| err("adapter_pin_invalid"))?;
    if !m.is_file()
        || !p.executable.is_absolute()
        || !hex64(&p.sha256)
        || digest_file(&p.executable)? != p.sha256
    {
        return Err(err("adapter_pin_invalid"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if m.permissions().mode() & 0o022 != 0 {
            return Err(err("adapter_pin_invalid"));
        }
    }
    Ok(())
}
fn private_dir(path: &Path) -> Result<()> {
    let m = std::fs::symlink_metadata(path).map_err(|_| err("adapter_state_invalid"))?;
    if !m.is_dir() || path.canonicalize().ok().as_deref() != Some(path) {
        return Err(err("adapter_state_invalid"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if m.permissions().mode() & 0o077 != 0 {
            return Err(err("adapter_state_invalid"));
        }
    }
    Ok(())
}
fn create_dir(path: &Path) -> Result<()> {
    match std::fs::create_dir(path) {
        Ok(()) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                    .map_err(|_| err("adapter_state_invalid"))?;
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(err("adapter_state_invalid")),
    }
    private_dir(path)
}
fn write_new(path: &Path, value: &Value) -> Result<()> {
    write_bytes(path, value.to_string().as_bytes())
}
fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut o = OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(path).map_err(|_| err("adapter_intent_exists"))?;
    f.write_all(bytes)
        .map_err(|_| err("adapter_state_invalid"))?;
    f.sync_all().map_err(|_| err("adapter_state_invalid"))?;
    std::fs::File::open(path.parent().unwrap())
        .and_then(|f| f.sync_all())
        .map_err(|_| err("adapter_state_invalid"))
}
fn read_value(path: &Path) -> Result<Value> {
    let m = std::fs::symlink_metadata(path).map_err(|_| err("adapter_state_invalid"))?;
    if !m.is_file() || m.len() > 131072 {
        return Err(err("adapter_state_invalid"));
    }
    serde_json::from_slice(&std::fs::read(path).map_err(|_| err("adapter_state_invalid"))?)
        .map_err(|_| err("adapter_state_invalid"))
}
fn run_dir(req: &Request) -> Result<PathBuf> {
    let dir = req.adapter_policy.state_root.join(&req.run_id);
    create_dir(&dir)?;
    let manifest = dir.join("run.json");
    if manifest.exists() {
        if read_value(&manifest)? != req.scope() {
            return Err(err("adapter_scope_changed"));
        }
    } else {
        write_new(&manifest, &req.scope())?;
    }
    Ok(dir)
}
async fn limited<R: AsyncRead + Unpin>(mut r: R, max: usize) -> std::result::Result<Vec<u8>, ()> {
    let mut v = Vec::new();
    let mut b = [0; 4096];
    loop {
        let n = r.read(&mut b).await.map_err(|_| ())?;
        if n == 0 {
            return Ok(v);
        }
        if v.len() + n > max {
            return Err(());
        }
        v.extend_from_slice(&b[..n]);
    }
}
pub(crate) async fn command(
    program: &Program,
    args: &[String],
    stdin: Option<&[u8]>,
    env: &[(&str, String)],
    timeout: Duration,
    max: usize,
) -> Result<Vec<u8>> {
    pin(program)?;
    let mut c = Command::new(&program.executable);
    c.args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GH_CONFIG_DIR", "/nonexistent-workboard-gh")
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (k, v) in env {
        c.env(k, v);
    }
    let mut child = c.spawn().map_err(|_| err("adapter_command_failed"))?;
    let mut input = child.stdin.take().unwrap();
    let out = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let data = stdin.unwrap_or(&[]).to_vec();
    let result = tokio::time::timeout(timeout, async {
        input
            .write_all(&data)
            .await
            .map_err(|_| err("adapter_command_failed"))?;
        input
            .shutdown()
            .await
            .map_err(|_| err("adapter_command_failed"))?;
        drop(input);
        let (status, out, stderr) = tokio::try_join!(
            async { child.wait().await.map_err(|_| ()) },
            limited(out, max),
            limited(stderr, max)
        )
        .map_err(|_| err("adapter_command_failed"))?;
        if !status.success() {
            let stage = crate::diagnostics::Failure::command_stage(
                program.executable.to_str().unwrap_or(""),
                args,
            );
            let failure = crate::diagnostics::Failure::parse(&stderr).unwrap_or_else(|| {
                crate::diagnostics::Failure::process(stage, status, &out, &stderr)
            });
            return Err(err("adapter_command_failed").with_diagnostic(failure));
        }
        Ok(out)
    })
    .await;
    if !matches!(result, Ok(Ok(_))) {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    result.map_err(|_| err("adapter_command_timeout"))?
}
fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}
async fn git(req: &Request, parts: &[&str], input: Option<&[u8]>) -> Result<String> {
    let mut a = args(&[
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "protocol.file.allow=never",
        "-C",
        req.workspace
            .to_str()
            .ok_or_else(|| err("adapter_request_invalid"))?,
    ]);
    a.extend(args(parts));
    String::from_utf8(
        command(
            &req.adapter_policy.git,
            &a,
            input,
            &[],
            Duration::from_secs(30),
            1048576,
        )
        .await?,
    )
    .map_err(|_| err("adapter_command_invalid"))
}
async fn docker(
    req: &Request,
    parts: Vec<String>,
    input: Option<&[u8]>,
    seconds: u64,
    max: usize,
) -> Result<Vec<u8>> {
    let mut a = args(&["--host", &req.adapter_policy.docker_host]);
    a.extend(parts);
    command(
        &req.adapter_policy.docker,
        &a,
        input,
        &[],
        Duration::from_secs(seconds.max(1)),
        max,
    )
    .await
}
async fn topology(req: &Request) -> Result<()> {
    let p = &req.adapter_policy;
    let network: Value = serde_json::from_slice(
        &docker(
            req,
            args(&["network", "inspect", &p.worker_network]),
            None,
            10,
            65536,
        )
        .await?,
    )
    .map_err(|_| err("adapter_network_invalid"))?;
    if network[0]["Id"] != p.worker_network || network[0]["Internal"] != true {
        return Err(err("adapter_network_invalid"));
    }
    let proxy: Value = serde_json::from_slice(
        &docker(req, args(&["inspect", &p.proxy_container]), None, 10, 65536).await?,
    )
    .map_err(|_| err("adapter_proxy_invalid"))?;
    let c = &proxy[0];
    let env = c["Config"]["Env"]
        .as_array()
        .ok_or_else(|| err("adapter_proxy_invalid"))?;
    for expected in [
        "CODEX_EGRESS_PROXY_ENABLED=true".to_string(),
        format!(
            "CODEX_EGRESS_ALLOWED_CONNECT_HOSTS={}",
            p.proxy_allowed_hosts.join(",")
        ),
        format!("CODEX_EGRESS_POLICY_SHA256={}", p.proxy_policy_sha256),
    ] {
        if !env.iter().any(|v| v.as_str() == Some(&expected)) {
            return Err(err("adapter_proxy_invalid"));
        }
    }
    if c["Id"] != p.proxy_container
        || c["Image"] != p.proxy_image
        || c["State"]["Running"] != true
        || !c["NetworkSettings"]["Networks"]
            .as_object()
            .is_some_and(|o| {
                o.values()
                    .any(|v| v["NetworkID"] == p.worker_network && v["IPAddress"] == p.proxy_ip)
            })
        || !c["Mounts"].as_array().is_some_and(|a| a.is_empty())
    {
        return Err(err("adapter_proxy_invalid"));
    }
    Ok(())
}
async fn containers(req: &Request) -> Result<Vec<String>> {
    let out = docker(
        req,
        args(&[
            "ps",
            "--all",
            "--quiet",
            "--no-trunc",
            "--filter",
            &format!("label=io.workboard.run={}", req.run_id),
        ]),
        None,
        10,
        65536,
    )
    .await?;
    let text = String::from_utf8(out).map_err(|_| err("adapter_cleanup_unknown"))?;
    let ids: Vec<_> = text
        .lines()
        .filter(|v| !v.is_empty())
        .map(String::from)
        .collect();
    if ids.iter().any(|s| !hex64(s)) || ids.len() > 8 {
        return Err(err("adapter_cleanup_unknown"));
    }
    Ok(ids)
}
fn container_name(req: &Request) -> String {
    format!("workboard-{}-{}", req.run_id, req.action_id)
}
pub async fn main_entry(role: &str) -> Result<()> {
    let raw = limited(tokio::io::stdin(), 65536)
        .await
        .map_err(|_| err("adapter_request_invalid"))?;
    let req: Request = serde_json::from_slice(&raw).map_err(|_| err("adapter_request_invalid"))?;
    req.validate()?;
    let dir = run_dir(&req)?;
    let action_dir = dir.join(&req.action_id);
    create_dir(&action_dir)?;
    let intent = action_dir.join("intent.json");
    let receipt_path = action_dir.join("receipt.json");
    let fingerprint = format!("{:x}", sha2::Sha256::digest(&raw));
    if intent.exists() {
        if read_value(&intent)?["fingerprint"] != fingerprint {
            return Err(err("adapter_action_conflict"));
        }
        if receipt_path.exists() {
            println!("{}", read_value(&receipt_path)?);
            return Ok(());
        }
        return Err(err("adapter_action_uncertain"));
    }
    write_new(
        &intent,
        &json!({"fingerprint":fingerprint,"action":req.action,"action_id":req.action_id,"run_id":req.run_id,"repository":req.repository,"container":if role=="worker" && req.action!="cancel"{Some(container_name(&req))}else{None},"at_ms":now_ms()}),
    )?;
    let result = if role == "worker"
        && req
            .adapter_policy
            .native_agent
            .as_ref()
            .is_some_and(|p| req.action != "plan" || p.native_planning)
    {
        native::run(&req, &action_dir).await
    } else if role == "worker" {
        worker::run(&req, &action_dir).await
    } else {
        publisher::run(&req, &action_dir).await
    };
    let receipt = match result {
        Ok(receipt) => receipt,
        Err(error) => {
            if let Some(failure) = error.diagnostic() {
                // Preserve the original diagnostic even if the private journal cannot be written.
                let _ = write_new(&action_dir.join("failure.json"), &failure.json());
            }
            return Err(error);
        }
    };
    let value = serde_json::to_value(receipt).map_err(|_| err("adapter_receipt_invalid"))?;
    write_new(&receipt_path, &value)?;
    println!("{value}");
    Ok(())
}
use sha2::Digest;
