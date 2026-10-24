//! Ticket repositories provision native-agent or legacy contained runtimes.
//! Legacy local-folder discovery remains a compatibility API, never a worker mount.
use crate::{
    ApiError, Result,
    execution_config::{Approval, Policy, digest_file},
    store::{Store, internal, now},
};
use hmac::{Hmac, Mac};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::Sha256;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Setup {
    schema_version: u32,
    runtime: Policy,
    max_runs_per_project: u32,
    #[serde(default)]
    planning_enabled: bool,
}
pub struct Factory {
    setup: Setup,
    path: PathBuf,
    digest: String,
    key: Arc<Vec<u8>>,
}
impl Factory {
    pub fn from_env() -> Result<Option<Self>> {
        let path = std::env::var_os("WORKBOARD_PROJECT_SETUP");
        let digest = std::env::var("WORKBOARD_PROJECT_SETUP_SHA256").ok();
        let key = std::env::var("WORKBOARD_PROJECT_SETUP_KEY").ok();
        if path.is_none() && digest.is_none() && key.is_none() {
            return Ok(None);
        }
        Self::load(
            Path::new(&path.ok_or_else(|| ApiError::bad("project_setup_unavailable"))?),
            key.ok_or_else(|| ApiError::bad("project_setup_unavailable"))?
                .as_bytes(),
            &digest.ok_or_else(|| ApiError::bad("project_setup_unavailable"))?,
        )
        .map(Some)
    }
    pub fn load(path: &Path, key: &[u8], digest: &str) -> Result<Self> {
        crate::execution_config::private_file(path)?;
        if !(32..=256).contains(&key.len()) || digest_file(path)? != digest {
            return Err(ApiError::bad("project_setup_unavailable"));
        }
        let setup: Setup = serde_json::from_slice(&std::fs::read(path).map_err(internal)?)
            .map_err(|_| ApiError::bad("project_setup_invalid"))?;
        if setup.schema_version != 1
            || !(1..=10000).contains(&setup.max_runs_per_project)
            || setup.runtime.ownership.is_some()
            || setup.runtime.adapter_policy.is_none()
        {
            return Err(ApiError::bad("project_setup_invalid"));
        }
        setup.runtime.adapter_policy.as_ref().unwrap().validate()?;
        Ok(Self {
            setup,
            path: path.into(),
            digest: digest.into(),
            key: Arc::new(key.to_vec()),
        })
    }
    pub(crate) fn verify(&self) -> Result<()> {
        crate::execution_config::private_file(&self.path)?;
        if digest_file(&self.path)? != self.digest
            || self.setup.runtime.expires_at_ms <= crate::execution_config::now_ms()
        {
            return Err(ApiError::unavailable("project_setup_expired"));
        }
        Ok(())
    }
    pub(crate) fn planning_available(&self) -> bool {
        self.setup.planning_enabled && self.verify().is_ok()
    }
    pub(crate) fn plan_approval(&self, id: &str, repository: &str) -> Result<Value> {
        if !self.planning_available() {
            return Err(ApiError::unavailable("planning_unavailable"));
        }
        let mut policy = self.setup.runtime.clone();
        policy.max_stage_seconds = policy.max_stage_seconds.min(300);
        policy.max_run_seconds = policy.max_run_seconds.min(300);
        policy.work_root = policy.work_root.join("planning");
        private_dir(&policy.work_root)?;
        let adapter = policy
            .adapter_policy
            .as_mut()
            .ok_or_else(ApiError::internal)?;
        adapter.repository = repository.into();
        if repository.is_empty() {
            if adapter.native_agent.is_none() {
                return Err(ApiError::unavailable("research_unavailable"));
            }
            adapter.research = true;
        }
        adapter.validation_profile = None;
        adapter.state_root = adapter.state_root.join("planning");
        private_dir(&adapter.state_root)?;
        let path = adapter.state_root.join(format!("approval-{id}.json"));
        let payload = serde_json::to_value(policy).map_err(internal)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).map_err(internal)?;
        mac.update(payload.to_string().as_bytes());
        let envelope =
            json!({"payload":payload,"signature":format!("{:x}",mac.finalize().into_bytes())});
        use std::io::Write;
        let mut file = crate::storage_ownership::private_options()
            .create_new(true)
            .open(&path)
            .map_err(internal)?;
        file.write_all(envelope.to_string().as_bytes())
            .map_err(internal)?;
        file.sync_all().map_err(internal)?;
        let digest = digest_file(&path)?;
        Approval::load(&path, &self.key, &digest)?.verify(&digest, false)?;
        Ok(json!({"approval_path":path,"approval_sha256":digest}))
    }
    pub(crate) fn research_available(&self) -> bool {
        self.verify().is_ok() && self.native_agents().is_some()
    }
    pub(crate) fn research_approval(&self, id: &str) -> Result<Approval> {
        if !self.research_available() {
            return Err(ApiError::unavailable("research_unavailable"));
        }
        let mut policy = self.setup.runtime.clone();
        let adapter = policy
            .adapter_policy
            .as_mut()
            .ok_or_else(ApiError::internal)?;
        adapter.repository = String::new();
        adapter.research = true;
        adapter.validation_profile = None;
        let path = adapter
            .state_root
            .join(format!("research-{id}-{}.json", uuid::Uuid::new_v4()));
        let payload = serde_json::to_value(policy).map_err(internal)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).map_err(internal)?;
        mac.update(payload.to_string().as_bytes());
        let value =
            json!({"payload":payload,"signature":format!("{:x}",mac.finalize().into_bytes())});
        use std::io::Write;
        let mut f = crate::storage_ownership::private_options()
            .create_new(true)
            .open(&path)
            .map_err(internal)?;
        f.write_all(value.to_string().as_bytes())
            .map_err(internal)?;
        f.sync_all().map_err(internal)?;
        Approval::load(&path, &self.key, &digest_file(&path)?)
    }
    pub(crate) fn native_agents(&self) -> Option<&crate::adapters::native::NativeAgentPolicy> {
        self.setup
            .runtime
            .adapter_policy
            .as_ref()?
            .native_agent
            .as_ref()
    }
    pub(crate) fn validate_registry(&self, registry: &crate::system::Registry) -> Result<()> {
        let mut enabled = registry.clone();
        for agent in &mut enabled.agents {
            agent.enabled = true;
        }
        let mut assignments = vec![enabled.assignment(None)?];
        for agent in &enabled.agents {
            if agent.role == "coder" {
                assignments.push(enabled.assignment(Some(
                    &json!({"coding_agent_id":agent.id,"qa_policy":"required"}),
                ))?);
            }
        }
        for assignment in assignments {
            let mut policy = self.setup.runtime.clone();
            assignment.apply(&mut policy)?;
            if serde_json::to_vec(&policy).map_err(internal)?.len() + 512 > 16384 {
                return Err(ApiError::bad("agent_config_too_large"));
            }
        }
        Ok(())
    }
    pub(crate) fn task_approval(
        &self,
        project: &Value,
        task: &Value,
        registry: &crate::system::Registry,
    ) -> Result<Approval> {
        self.verify()?;
        self.approval(project)?
            .with_agents(registry.assignment(Some(task))?)
    }
    pub fn approval(&self, project: &Value) -> Result<Approval> {
        // Observation of an expired project's original approval remains possible.
        Approval::load(
            Path::new(
                project["approval_path"]
                    .as_str()
                    .ok_or_else(ApiError::internal)?,
            ),
            &self.key,
            project["approval_sha256"]
                .as_str()
                .ok_or_else(ApiError::internal)?,
        )
    }
    async fn git(&self, root: &Path, args: &[&str]) -> Result<String> {
        let root = root
            .to_str()
            .ok_or_else(|| ApiError::bad("project_path_invalid"))?;
        let mut argv = [
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-C",
            root,
        ]
        .map(str::to_string)
        .to_vec();
        argv.extend(args.iter().map(|s| s.to_string()));
        let out = crate::adapters::command(
            &self.setup.runtime.git,
            &argv,
            None,
            &[("GIT_OPTIONAL_LOCKS", "0".into())],
            std::time::Duration::from_secs(10),
            65536,
        )
        .await
        .map_err(|_| ApiError::bad("project_inspection_failed"))?;
        String::from_utf8(out).map_err(|_| ApiError::bad("project_inspection_failed"))
    }
    pub(crate) async fn github_session_token(&self) -> Result<String> {
        self.verify()?;
        let home = std::env::var("HOME")
            .map_err(|_| ApiError::unavailable("project_github_access_missing"))?;
        let config =
            std::env::var("GH_CONFIG_DIR").unwrap_or_else(|_| format!("{home}/.config/gh"));
        let argv = ["auth", "token", "--hostname", "github.com"].map(str::to_string);
        let bytes = crate::adapters::command(
            &self.setup.runtime.adapter_policy.as_ref().unwrap().gh,
            &argv,
            None,
            &[("HOME", home), ("GH_CONFIG_DIR", config)],
            std::time::Duration::from_secs(10),
            8192,
        )
        .await
        .map_err(|_| ApiError::unavailable("project_github_access_missing"))?;
        let value = String::from_utf8(bytes).map_err(internal)?;
        let token = value.trim();
        if token.is_empty() || token.chars().any(char::is_control) {
            return Err(ApiError::unavailable("project_github_access_missing"));
        }
        Ok(token.into())
    }
    async fn github(&self, path: &str) -> Result<Value> {
        let token = std::env::var("GH_TOKEN")
            .or_else(|_| std::env::var("GITHUB_TOKEN"))
            .unwrap_or_default();
        if token.is_empty() && !self.setup.runtime.publisher.credential_env_names.is_empty() {
            return Err(ApiError::unavailable("project_github_access_missing"));
        }
        let argv = ["api", "--hostname", "github.com", path].map(str::to_string);
        let out = crate::adapters::command(
            &self.setup.runtime.adapter_policy.as_ref().unwrap().gh,
            &argv,
            None,
            &[("GH_TOKEN", token)],
            std::time::Duration::from_secs(30),
            1048576,
        )
        .await
        .map_err(|_| ApiError::unavailable("project_github_check_failed"))?;
        serde_json::from_slice(&out)
            .map_err(|_| ApiError::unavailable("project_github_check_failed"))
    }
    pub async fn prepare(&self, input: &Value) -> Result<Value> {
        self.prepare_inner(input, None).await
    }
    pub async fn reconnect(&self, project: &Value) -> Result<Value> {
        if project["local_path"]
            .as_str()
            .is_some_and(|p| p.starts_with("repository:"))
        {
            return self
                .prepare_repository(project["repository"].as_str().unwrap_or(""), project)
                .await;
        }
        self.prepare_inner(&json!({"local_path":project["local_path"]}), Some(project))
            .await
    }
    async fn prepare_inner(&self, input: &Value, previous: Option<&Value>) -> Result<Value> {
        self.verify()?;
        if !input
            .as_object()
            .is_some_and(|o| o.len() == 1 && o.contains_key("local_path"))
        {
            return Err(ApiError::bad("request_invalid"));
        }
        let path = PathBuf::from(
            input["local_path"]
                .as_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| ApiError::bad("project_path_invalid"))?,
        );
        if !path.is_absolute()
            || path.canonicalize().ok().as_ref() != Some(&path)
            || !path
                .join(".git")
                .symlink_metadata()
                .is_ok_and(|m| m.is_dir() || (m.is_file() && m.len() <= 4096))
        {
            return Err(ApiError::bad("project_path_invalid"));
        }
        if self
            .git(&path, &["rev-parse", "--show-toplevel"])
            .await?
            .trim()
            != path.to_string_lossy()
        {
            return Err(ApiError::bad("project_path_invalid"));
        }
        let config = self.git(&path, &["config", "--null", "--list"]).await?;
        if config.split('\0').any(|entry| {
            entry
                .split('\n')
                .next()
                .is_some_and(|key| key.starts_with("filter."))
        }) {
            return Err(ApiError::bad("project_filter_unsupported"));
        }
        if !self
            .git(&path, &["ls-files", "--", "dist"])
            .await?
            .is_empty()
        {
            return Err(ApiError::bad("project_profile_unsupported"));
        }
        if !self
            .git(
                &path,
                &["status", "--porcelain", "--untracked-files=normal"],
            )
            .await?
            .is_empty()
        {
            return Err(ApiError::conflict("project_source_dirty"));
        }
        let origin = self
            .git(&path, &["config", "--get", "remote.origin.url"])
            .await?;
        let repository = origin
            .trim()
            .strip_prefix("https://github.com/")
            .or_else(|| origin.trim().strip_prefix("git@github.com:"))
            .ok_or_else(|| ApiError::bad("invalid_repository"))?
            .trim_end_matches(".git");
        if !crate::store::valid_repository(repository) {
            return Err(ApiError::bad("invalid_repository"));
        }
        let remote = self.github(&format!("repos/{repository}")).await?;
        if remote["private"] != true
            && self
                .setup
                .runtime
                .adapter_policy
                .as_ref()
                .is_none_or(|p| p.native_agent.is_none())
        {
            return Err(ApiError::bad("project_private_repository_required"));
        }
        if remote["full_name"] != repository
            || remote["archived"] != false
            || remote["permissions"]["push"] != true
            || remote["id"].as_u64().is_none()
            || !remote["default_branch"].as_str().is_some_and(|s| {
                !s.is_empty()
                    && s.len() < 128
                    && !s.starts_with('-')
                    && !s.contains("..")
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_/.".contains(&b))
            })
        {
            return Err(ApiError::bad("project_github_permission_required"));
        }
        if previous.is_some_and(|p| {
            p["repository"] != repository
                || p["repository_id"] != remote["id"]
                || p["default_branch"] != remote["default_branch"]
        }) {
            return Err(ApiError::conflict("project_remote_changed"));
        }
        let manifest = path.join("package.json");
        let build = path.join("scripts/build.js");
        for p in [&manifest, &build] {
            if !p
                .symlink_metadata()
                .is_ok_and(|m| m.is_file() && m.len() <= 1048576)
                || p.canonicalize().ok().as_ref() != Some(p)
            {
                return Err(ApiError::bad("project_profile_unsupported"));
            }
        }
        let package: Value = serde_json::from_slice(&std::fs::read(&manifest).map_err(internal)?)
            .map_err(|_| ApiError::bad("project_profile_unsupported"))?;
        if package["scripts"]["test"] != "node --test test/*.test.js"
            || package["scripts"]["build"] != "node scripts/build.js"
        {
            return Err(ApiError::bad("project_profile_unsupported"));
        }
        for name in ["package.json", "scripts/build.js"] {
            let remote_file = self
                .github(&format!(
                    "repos/{repository}/contents/{name}?ref={}",
                    remote["default_branch"].as_str().unwrap()
                ))
                .await?;
            let local_blob = self
                .git(&path, &["hash-object", "--no-filters", "--", name])
                .await?;
            if remote_file["type"] != "file"
                || !remote_file["sha"]
                    .as_str()
                    .is_some_and(crate::execution_config::sha)
                || remote_file["sha"] != local_blob.trim()
            {
                return Err(ApiError::conflict("project_profile_sync_required"));
            }
        }
        let profile = crate::adapters::ValidationProfile::NodeBuiltin {
            manifest_sha256: digest_file(&manifest)?,
            build_sha256: digest_file(&build)?,
        };
        self.provision(
            repository,
            &remote,
            Some(profile),
            previous,
            Some(&path),
            None,
        )
    }
    /// Discover only GitHub's authenticated current default branch. No canonical
    /// user checkout is read or mounted, and no project code runs on the host.
    pub async fn prepare_repository(&self, repository: &str, previous: &Value) -> Result<Value> {
        self.verify()?;
        if !crate::store::valid_repository(repository) {
            return Err(ApiError::bad("invalid_repository"));
        }
        let remote = self.github(&format!("repos/{repository}")).await?;
        let canonical = remote["full_name"]
            .as_str()
            .filter(|s| s.eq_ignore_ascii_case(repository) && crate::store::valid_repository(s))
            .ok_or_else(|| ApiError::bad("project_remote_changed"))?;
        if remote["private"] != true
            && self
                .setup
                .runtime
                .adapter_policy
                .as_ref()
                .is_none_or(|p| p.native_agent.is_none())
        {
            return Err(ApiError::bad("project_private_repository_required"));
        }
        let branch = remote["default_branch"]
            .as_str()
            .filter(|s| {
                !s.is_empty()
                    && s.len() < 128
                    && !s.starts_with('-')
                    && !s.contains("..")
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_/.".contains(&b))
            })
            .ok_or_else(|| ApiError::bad("project_remote_changed"))?;
        if remote["archived"] != false
            || remote["permissions"]["push"] != true
            || remote["id"].as_u64().is_none()
        {
            return Err(ApiError::bad("project_github_permission_required"));
        }
        if !previous["repository_id"].is_null() && previous["repository_id"] != remote["id"] {
            return Err(ApiError::conflict("project_remote_changed"));
        }
        crate::adapters::pin(&self.setup.runtime.publisher)?;
        let adapter = self.setup.runtime.adapter_policy.as_ref().unwrap();
        if adapter.native_agent.is_some() {
            return self
                .provision(canonical, &remote, None, Some(previous), None, None)
                .map(|mut p| {
                    p["checked_task_revision"] = previous["checked_task_revision"].clone();
                    p
                });
        }
        let discovery = adapter
            .state_root
            .join(format!("discovery-{}", uuid::Uuid::new_v4()));
        private_dir(&discovery)?;
        let _cleanup = DirectoryCleanup(discovery.clone());
        let source = discovery.join("source");
        let argv = [
            "-c",
            "credential.helper=",
            "-c",
            "credential.useHttpPath=true",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "protocol.file.allow=never",
            "clone",
            "--depth=1",
            "--single-branch",
            "--branch",
            branch,
            "--",
            &format!("https://github.com/{canonical}.git"),
            source.to_str().unwrap(),
        ]
        .map(str::to_string);
        crate::adapters::command(
            &self.setup.runtime.git,
            &argv,
            None,
            &[
                (
                    "GH_TOKEN",
                    std::env::var("GH_TOKEN")
                        .or_else(|_| std::env::var("GITHUB_TOKEN"))
                        .unwrap_or_default(),
                ),
                (
                    "GIT_ASKPASS",
                    self.setup
                        .runtime
                        .publisher
                        .executable
                        .to_string_lossy()
                        .into(),
                ),
                ("WORKBOARD_GIT_REPOSITORY", canonical.into()),
            ],
            std::time::Duration::from_secs(120),
            1048576,
        )
        .await
        .map_err(|_| ApiError::unavailable("project_clone_failed"))?;
        let package = source.join("package.json");
        let raw = regular_bytes(&package, 1048576)?;
        let manifest: Value = serde_json::from_slice(&raw)
            .map_err(|_| ApiError::bad("project_profile_unsupported"))?;
        let (profile, image) = if manifest["scripts"]["test"] == "node --test test/*.test.js"
            && manifest["scripts"]["build"] == "node scripts/build.js"
        {
            (
                crate::adapters::ValidationProfile::NodeBuiltin {
                    manifest_sha256: digest_file(&package)?,
                    build_sha256: {
                        use sha2::Digest;
                        format!(
                            "{:x}",
                            Sha256::digest(regular_bytes(&source.join("scripts/build.js"), 65536)?)
                        )
                    },
                },
                None,
            )
        } else if manifest["scripts"]["test"] == "node --test"
            && manifest["scripts"]["build"].is_null()
        {
            let profile = crate::adapters::ValidationProfile::NodeUnit {
                manifest_sha256: digest_file(&package)?,
            };
            crate::adapters::validate_unit_source(&source, &profile)?;
            (profile, None)
        } else if manifest["scripts"]["test"] == "node --test test/*.test.js"
            && manifest["scripts"]["build"] == "vite build"
            && manifest["scripts"]["check:types"] == "tsc --noEmit"
        {
            let lock = regular_bytes(&source.join("package-lock.json"), 4194304)?;
            crate::adapters::validate_vite_inputs(&raw, &lock)?;
            let prior = self.approval(previous).ok();
            let reused = prior.filter(|a| {
                previous["setup_sha256"] == self.digest
                    && previous["profile"]["id"] == "node-vite-v1"
                    && previous["profile"]["manifest_sha256"]
                        == digest_file(&package).unwrap_or_default()
                    && previous["profile"]["lock_sha256"]
                        == digest_file(&source.join("package-lock.json")).unwrap_or_default()
                    && a.verify(&a.digest, false).is_ok()
            });
            let (image, cache) = if let Some(a) = reused {
                (
                    a.policy.adapter_policy.unwrap().worker_image,
                    previous["profile"]["cache_sha256"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                )
            } else {
                self.prepare_cache(&raw, &lock).await?
            };
            (
                crate::adapters::ValidationProfile::NodeVite {
                    manifest_sha256: digest_file(&package)?,
                    lock_sha256: digest_file(&source.join("package-lock.json"))?,
                    cache_sha256: cache,
                },
                Some(image),
            )
        } else {
            return Err(ApiError::bad("project_profile_unsupported"));
        };
        let result = self
            .provision(
                canonical,
                &remote,
                Some(profile),
                Some(previous),
                None,
                image.as_deref(),
            )
            .map(|mut p| {
                p["checked_task_revision"] = previous["checked_task_revision"].clone();
                p
            });
        let _ = std::fs::remove_dir_all(&discovery);
        result
    }
    async fn docker(&self, args: Vec<String>, timeout: u64) -> Result<Vec<u8>> {
        let p = self.setup.runtime.adapter_policy.as_ref().unwrap();
        let config = p
            .state_root
            .join(format!("docker-client-{}", uuid::Uuid::new_v4()));
        private_dir(&config)?;
        let _cleanup = DirectoryCleanup(config.clone());
        let plugins = p
            .docker
            .executable
            .parent()
            .and_then(Path::parent)
            .map(|p| p.join("cli-plugins"));
        let config_value = if let Some(plugins) = plugins.filter(|p| {
            p.join("docker-buildx")
                .symlink_metadata()
                .is_ok_and(|m| m.is_file())
                && p.canonicalize().ok().as_ref() == Some(p)
        }) {
            json!({"cliPluginsExtraDirs":[plugins]})
        } else {
            json!({})
        };
        let config_file = config.join("config.json");
        use std::io::Write;
        let mut f = crate::storage_ownership::private_options()
            .create_new(true)
            .open(&config_file)
            .map_err(internal)?;
        f.write_all(config_value.to_string().as_bytes())
            .map_err(internal)?;
        f.sync_all().map_err(internal)?;
        drop(f);
        let mut argv = vec![
            "--config".into(),
            config.to_string_lossy().into(),
            "--host".into(),
            p.docker_host.clone(),
        ];
        argv.extend(args);
        crate::adapters::command(
            &p.docker,
            &argv,
            None,
            &[],
            std::time::Duration::from_secs(timeout),
            1048576,
        )
        .await
        .map_err(|_| ApiError::unavailable("project_cache_build_failed"))
    }
    async fn prepare_cache(&self, package: &[u8], lock: &[u8]) -> Result<(String, String)> {
        use sha2::Digest;
        let p = self.setup.runtime.adapter_policy.as_ref().unwrap();
        let context = p
            .state_root
            .join(format!("cache-build-{}", uuid::Uuid::new_v4()));
        private_dir(&context)?;
        let _cleanup = DirectoryCleanup(context.clone());
        for (name, bytes) in [
            ("package.json", package),
            ("package-lock.json", lock),
            (
                "Dockerfile",
                include_bytes!("../containers/codex/Dockerfile.node-vite").as_slice(),
            ),
            (
                "prepare-cache.mjs",
                include_bytes!("../containers/codex/prepare-node-cache.mjs").as_slice(),
            ),
        ] {
            let mut f = crate::storage_ownership::private_options()
                .create_new(true)
                .open(context.join(name))
                .map_err(internal)?;
            use std::io::Write;
            f.write_all(bytes).map_err(internal)?;
            f.sync_all().map_err(internal)?;
        }
        let base = String::from_utf8(
            self.docker(
                vec![
                    "image".into(),
                    "inspect".into(),
                    "--format={{.Id}}".into(),
                    p.worker_image.clone(),
                ],
                15,
            )
            .await?,
        )
        .map_err(internal)?;
        if base.trim() != p.worker_image {
            return Err(ApiError::bad("project_cache_build_failed"));
        }
        let base_tag = format!("workboard-cache-base:{}", uuid::Uuid::new_v4());
        let tag = format!("workboard-cache:{}", uuid::Uuid::new_v4());
        self.docker(
            vec![
                "image".into(),
                "tag".into(),
                p.worker_image.clone(),
                base_tag.clone(),
            ],
            15,
        )
        .await?;
        let result = self
            .docker(
                vec![
                    "build".into(),
                    "--quiet".into(),
                    "--pull=false".into(),
                    "--build-arg".into(),
                    format!("WORKER_IMAGE={base_tag}"),
                    "--tag".into(),
                    tag.clone(),
                    context.to_string_lossy().into(),
                ],
                600,
            )
            .await;
        let after = self
            .docker(
                vec![
                    "image".into(),
                    "inspect".into(),
                    "--format={{.Id}}".into(),
                    base_tag.clone(),
                ],
                15,
            )
            .await?;
        let _ = self
            .docker(vec!["image".into(), "rm".into(), base_tag], 15)
            .await;
        result?;
        if String::from_utf8(after).map_err(internal)?.trim() != p.worker_image {
            return Err(ApiError::bad("project_cache_build_failed"));
        }
        let image = String::from_utf8(
            self.docker(
                vec![
                    "image".into(),
                    "inspect".into(),
                    "--format={{.Id}}".into(),
                    tag,
                ],
                15,
            )
            .await?,
        )
        .map_err(internal)?
        .trim()
        .to_string();
        if !image
            .strip_prefix("sha256:")
            .is_some_and(|v| crate::execution_config::decode(v).is_some())
        {
            return Err(ApiError::bad("project_cache_build_failed"));
        }
        let marker = self
            .docker(
                vec![
                    "run".into(),
                    "--rm".into(),
                    "--network=none".into(),
                    "--read-only".into(),
                    "--cap-drop=ALL".into(),
                    "--security-opt=no-new-privileges".into(),
                    "--entrypoint=/bin/cat".into(),
                    image.clone(),
                    "/opt/workboard-node-profile/cache.json".into(),
                ],
                30,
            )
            .await?;
        let value: Value = serde_json::from_slice(&marker)
            .map_err(|_| ApiError::bad("project_cache_build_failed"))?;
        let hash = |b: &[u8]| format!("{:x}", Sha256::digest(b));
        if value["id"] != "node-vite-v1"
            || value["manifest_sha256"] != hash(package)
            || value["lock_sha256"] != hash(lock)
            || !value["cache_files_sha256"]
                .as_str()
                .is_some_and(|v| crate::execution_config::decode(v).is_some())
        {
            return Err(ApiError::bad("project_cache_build_failed"));
        }
        let _ = std::fs::remove_dir_all(context);
        Ok((image, hash(&marker)))
    }
    fn provision(
        &self,
        repository: &str,
        remote: &Value,
        profile: Option<crate::adapters::ValidationProfile>,
        previous: Option<&Value>,
        path: Option<&Path>,
        image: Option<&str>,
    ) -> Result<Value> {
        let summary = if profile.is_none() {
            "Assigned agent discovers the environment and selects actual project checks"
        } else if matches!(
            profile,
            Some(crate::adapters::ValidationProfile::NodeUnit { .. })
        ) {
            "Node unit tests · JavaScript syntax (no install or app build)"
        } else if matches!(
            profile,
            Some(crate::adapters::ValidationProfile::NodeVite { .. })
        ) {
            "Node tests · TypeScript check · Vite build"
        } else {
            "Node tests · review UI build"
        };
        let id = previous
            .and_then(|p| p["id"].as_str())
            .map(str::to_string)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let mut policy = self.setup.runtime.clone();
        policy.work_root = policy.work_root.join(&id);
        private_dir(&policy.work_root)?;
        let adapter = policy.adapter_policy.as_mut().unwrap();
        adapter.repository = repository.into();
        adapter.validation_profile = profile.clone();
        adapter.state_root = adapter.state_root.join(&id);
        private_dir(&adapter.state_root)?;
        if let Some(image) = image {
            adapter.worker_image = image.into();
        }
        let approval_path = adapter
            .state_root
            .join(format!("execution-{}.json", uuid::Uuid::new_v4()));
        let payload = serde_json::to_value(&policy).map_err(internal)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).map_err(internal)?;
        mac.update(payload.to_string().as_bytes());
        let envelope =
            json!({"payload":payload,"signature":format!("{:x}",mac.finalize().into_bytes())});
        use std::io::Write;
        let mut f = crate::storage_ownership::private_options()
            .create_new(true)
            .open(&approval_path)
            .map_err(internal)?;
        f.write_all(envelope.to_string().as_bytes())
            .map_err(internal)?;
        f.sync_all().map_err(internal)?;
        let approval_digest = digest_file(&approval_path)?;
        Approval::load(&approval_path, &self.key, &approval_digest)?
            .verify(&approval_digest, false)?;
        Ok(
            json!({"id":id,"revision":previous.map(|p|p["revision"].as_u64().unwrap_or(0)+1).unwrap_or(1),"name":path.and_then(|p|p.file_name()).map(|s|s.to_string_lossy().into_owned()).unwrap_or_else(||repository.into()),"repository":repository,"repository_id":remote["id"],"default_branch":remote["default_branch"],"local_path":path.map(|p|p.to_string_lossy().into_owned()).unwrap_or_else(||format!("repository:{}",repository.to_lowercase())),"profile":profile,"validation_summary":summary,"paused":path.is_some(),"failure":null,"lane_run_id":null,"runtime_status":"configured","approval_path":approval_path,"approval_sha256":approval_digest,"setup_sha256":self.digest,"max_runs":self.setup.max_runs_per_project,"created_at":previous.map(|p|p["created_at"].clone()).unwrap_or_else(||json!(now())),"updated_at":now()}),
        )
    }
}
fn private_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            use std::os::unix::fs::PermissionsExt;
            let m = path.symlink_metadata().map_err(internal)?;
            if !m.is_dir()
                || m.permissions().mode() & 0o077 != 0
                || path.canonicalize().ok().as_deref() != Some(path)
            {
                return Err(ApiError::bad("project_runtime_unavailable"));
            }
            Ok(())
        }
        Err(e) => Err(internal(e)),
    }
}
pub fn public(mut project: Value) -> Value {
    if let Some(o) = project.as_object_mut() {
        for k in [
            "approval_path",
            "approval_sha256",
            "setup_sha256",
            "max_runs",
        ] {
            o.remove(k);
        }
    }
    project
}
pub(crate) fn by_repository(db: &rusqlite::Connection, repository: &str) -> Result<Option<Value>> {
    let raw: Option<String> = db
        .query_row(
            "SELECT document FROM projects WHERE repository=lower(?1)",
            [repository],
            |r| r.get(0),
        )
        .optional()
        .map_err(internal)?;
    raw.map(|s| serde_json::from_str(&s).map_err(internal))
        .transpose()
}
pub(crate) fn get(db: &rusqlite::Connection, id: &str) -> Result<Value> {
    let raw: Option<String> = db
        .query_row("SELECT document FROM projects WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()
        .map_err(internal)?;
    serde_json::from_str(&raw.ok_or_else(|| ApiError::new(404, "project_not_found"))?)
        .map_err(internal)
}
pub(crate) fn save(db: &rusqlite::Connection, p: &Value) -> Result<()> {
    db.execute("INSERT INTO projects(id,repository,local_path,document) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET document=excluded.document",params![p["id"].as_str(),p["repository"].as_str().unwrap_or("").to_lowercase(),p["local_path"].as_str(),p.to_string()]).map_err(|_|ApiError::conflict("project_already_exists"))?;
    Ok(())
}
impl Store {
    pub(crate) fn project_update(&mut self, p: &Value) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(internal)?;
        save(&tx, p)?;
        tx.execute(
            "INSERT INTO events(task_id,document) VALUES(NULL,?1)",
            [json!({"type":"project.updated","project_id":p["id"],"at":now()}).to_string()],
        )
        .map_err(internal)?;
        tx.commit().map_err(internal)
    }
    pub(crate) fn project_fault(&mut self, id: &str, code: &str) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(internal)?;
        let mut p = get(&tx, id)?;
        p["paused"] = json!(true);
        p["failure"] = json!(code);
        if p["runtime_status"] == "preparing" {
            p["runtime_status"] = json!("failed");
        }
        p["revision"] = json!(p["revision"].as_u64().unwrap_or(0) + 1);
        p["updated_at"] = json!(now());
        save(&tx, &p)?;
        tx.execute(
            "INSERT INTO events(task_id,document) VALUES(NULL,?1)",
            [json!({"type":"project.updated","project_id":id,"at":now()}).to_string()],
        )
        .map_err(internal)?;
        tx.commit().map_err(internal)
    }
    pub(crate) fn projects(&self) -> Result<Vec<Value>> {
        let mut q = self
            .db
            .prepare("SELECT document FROM projects ORDER BY rowid")
            .map_err(internal)?;
        q.query_map([], |r| r.get::<_, String>(0))
            .map_err(internal)?
            .map(|r| serde_json::from_str(&r.map_err(internal)?).map_err(internal))
            .collect()
    }
    pub(crate) fn project_command(
        &mut self,
        key: &str,
        fingerprint: &str,
        id: Option<&str>,
        input: Value,
        prepared: Option<Value>,
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
        let creating = id.is_none();
        let mut p = if let Some(id) = id {
            get(&tx, id)?
        } else {
            prepared
                .clone()
                .ok_or_else(|| ApiError::unavailable("project_setup_unavailable"))?
        };
        if !creating && input["reconnect"] == true {
            if !input
                .as_object()
                .is_some_and(|o| o.len() == 2 && o.contains_key("expected_revision"))
                || input["expected_revision"] != p["revision"]
            {
                return Err(ApiError::conflict("revision_conflict"));
            }
            if !p["lane_run_id"].is_null() {
                return Err(ApiError::conflict("project_needs_reconciliation"));
            }
            p = prepared.ok_or_else(|| ApiError::unavailable("project_setup_unavailable"))?;
            let repository_mode = p["local_path"]
                .as_str()
                .is_some_and(|s| s.starts_with("repository:"));
            if repository_mode {
                p["paused"] = json!(false);
                p["failure"] = Value::Null;
                p["checked_task_revision"] = Value::Null;
                p["runtime_status"] = json!("pending");
                p["revision"] = json!(p["revision"].as_u64().unwrap_or(0) + 1);
                p["updated_at"] = json!(now());
            }
            let rows = {
                let mut q = tx
                    .prepare(
                        "SELECT document FROM tasks WHERE json_extract(document,'$.project_id')=?1",
                    )
                    .map_err(internal)?;
                q.query_map([p["id"].as_str()], |r| r.get::<_, String>(0))
                    .map_err(internal)?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(internal)?
            };
            for raw in rows {
                let mut t: Value = serde_json::from_str(&raw).map_err(internal)?;
                if !repository_mode && t["automation_policy"] == "ready_to_done" {
                    t["automation_policy"] = json!("manual");
                    t["automation_revision"] = Value::Null;
                    t["revision"] = json!(t["revision"].as_u64().unwrap() + 1);
                    t["updated_at"] = json!(now());
                    crate::store::persist(&tx, &t, "task.updated")?;
                }
            }
        } else if !creating {
            if !input.as_object().is_some_and(|o| {
                o.len() == 2 && o.contains_key("paused") && o.contains_key("expected_revision")
            }) || !input["paused"].is_boolean()
            {
                return Err(ApiError::bad("request_invalid"));
            }
            if input["expected_revision"] != p["revision"] {
                return Err(ApiError::conflict("revision_conflict"));
            }
            if input["paused"] == false {
                if p["runtime_status"] == "failed" {
                    return Err(ApiError::conflict("project_needs_preparation"));
                }
                if let Some(run) = p["lane_run_id"].as_str() {
                    let raw: String = tx
                        .query_row("SELECT document FROM runs WHERE id=?1", [run], |r| r.get(0))
                        .map_err(internal)?;
                    let run: Value = serde_json::from_str(&raw).map_err(internal)?;
                    if matches!(run["status"].as_str(), Some("unknown" | "cancel_requested")) {
                        return Err(ApiError::conflict("project_needs_reconciliation"));
                    }
                }
            }
            p["paused"] = input["paused"].clone();
            if p["paused"] == false {
                p["failure"] = Value::Null;
            }
            p["revision"] = json!(p["revision"].as_u64().unwrap() + 1);
            p["updated_at"] = json!(now());
        }
        save(&tx, &p)?;
        let event = json!({"type":"project.updated","project_id":p["id"],"at":now()});
        tx.execute(
            "INSERT INTO events(task_id,document) VALUES(NULL,?1)",
            [event.to_string()],
        )
        .map_err(internal)?;
        let status = if creating { 201 } else { 200 };
        let response = json!({"project":public(p)});
        tx.execute(
            "INSERT INTO commands VALUES(?1,?2,?3,?4)",
            params![key, fingerprint, status, response.to_string()],
        )
        .map_err(internal)?;
        tx.commit().map_err(internal)?;
        Ok((status, response))
    }
}

fn regular_bytes(path: &Path, max: u64) -> Result<Vec<u8>> {
    let m = path
        .symlink_metadata()
        .map_err(|_| ApiError::bad("project_profile_unsupported"))?;
    if !m.is_file() || m.len() > max || path.canonicalize().ok().as_deref() != Some(path) {
        return Err(ApiError::bad("project_profile_unsupported"));
    }
    std::fs::read(path).map_err(internal)
}

struct DirectoryCleanup(PathBuf);
impl Drop for DirectoryCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
