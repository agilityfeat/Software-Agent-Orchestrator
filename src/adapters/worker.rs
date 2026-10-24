use super::*;
use std::os::unix::fs::PermissionsExt;

pub(super) async fn run(req: &Request, action_dir: &Path) -> Result<Receipt> {
    if let Some(profile) = req
        .adapter_policy
        .validation_profile
        .as_ref()
        .filter(|p| p.node() && ["coder", "qa"].contains(&req.action.as_str()))
    {
        profiles::project_pins(&req.workspace, profile)?;
    }
    let result = run_inner(req, action_dir).await;
    if result.is_err() && req.action != "cancel" {
        let _ = cancel(req).await;
    }
    result
}
async fn run_inner(req: &Request, action_dir: &Path) -> Result<Receipt> {
    if req.action == "cancel" {
        cancel(req).await?;
        return Ok(req.receipt());
    }
    if !["coder", "qa", "plan"].contains(&req.action.as_str()) {
        return Err(err("adapter_action_invalid"));
    }
    topology(req).await?;
    if !containers(req).await?.is_empty() {
        return Err(err("adapter_cleanup_unknown"));
    }
    if req.stage_deadline_ms <= now_ms()
        || req.stage_deadline_ms > req.deadline_ms
        || req.stage_deadline_ms > now_ms() + 3600000
    {
        return Err(err("adapter_deadline_exceeded"));
    }
    if std::env::var_os("CODEX_HOME").as_deref() != Some(req.adapter_policy.auth_home.as_os_str()) {
        return Err(err("adapter_auth_reference_invalid"));
    }
    let auth = req.adapter_policy.auth_home.join("auth.json");
    let (auth_bytes, initial_meta) = read_auth(&auth)?;
    write_new(&action_dir.join("auth-reference.json"), &initial_meta)?;
    if req.action != "plan" {
        let before = git(req, &["rev-parse", "HEAD"], None).await?;
        if Some(before.trim()) != req.head_sha.as_deref() {
            return Err(err("adapter_head_changed"));
        }
    }
    let name = container_name(req);
    let (tmp_mount, workspace_mount) =
        temporary_filesystems(req.adapter_policy.validation_profile.as_ref());
    let mut create = args(&[
        "create",
        "--name",
        &name,
        "--read-only",
        "--user",
        "65532:65532",
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges",
        "--pids-limit",
        "128",
        "--log-driver",
        "none",
        "--network",
        &req.adapter_policy.worker_network,
        "--dns",
        "127.0.0.1",
        "--add-host",
        &format!("provider-proxy:{}", req.adapter_policy.proxy_ip),
        "--cpus",
        &req.adapter_policy.cpus.to_string(),
        "--memory",
        &format!("{}m", req.adapter_policy.memory_mb),
        "--tmpfs",
        tmp_mount,
        "--env",
        "HOME=/tmp/home",
        "--env",
        "CODEX_HOME=/tmp/codex",
        "--env",
        "PYTHONDONTWRITEBYTECODE=1",
        "--env",
        "HTTPS_PROXY=http://provider-proxy:3128",
        "--env",
        "HTTP_PROXY=http://provider-proxy:3128",
        "--env",
        "ALL_PROXY=http://provider-proxy:3128",
        "--label",
        &format!("io.workboard.run={}", req.run_id),
        "--label",
        &format!("io.workboard.action={}", req.action_id),
        "--label",
        &format!("io.workboard.approval={}", req.approval_sha256),
    ]);
    let source = req
        .workspace
        .to_str()
        .ok_or_else(|| err("adapter_request_invalid"))?;
    if source.contains(',') {
        return Err(err("adapter_request_invalid"));
    }
    if req.action == "plan" {
        create.extend(args(&[
            "--tmpfs",
            "/workspace:ro,noexec,nosuid,nodev,size=1048576,mode=0755",
        ]));
    } else if req.action == "coder" || profiles::node(&req.adapter_policy.validation_profile) {
        create.extend(args(&[
            "--mount",
            &format!("type=bind,source={source},target=/source,readonly"),
            "--tmpfs",
            workspace_mount,
        ]));
    } else {
        create.extend(args(&[
            "--mount",
            &format!("type=bind,source={source},target=/workspace,readonly"),
        ]));
    }
    create.extend(args(&[
        "--entrypoint",
        "/usr/local/bin/workboard-contained",
        &req.adapter_policy.worker_image,
        "--watchdog",
        &req.stage_deadline_ms.to_string(),
    ]));
    write_new(
        &action_dir.join("container-policy.json"),
        &json!({"image":req.adapter_policy.worker_image,"network":req.adapter_policy.worker_network,"role":req.action,"user":"65532:65532","readonly_root":true,"source_readonly":true,"source_inspected":req.action!="plan","source_mounted":req.action!="plan","validation_profile":req.adapter_policy.validation_profile,"workspace_disposable":profiles::node(&req.adapter_policy.validation_profile),"workspace_executable":matches!(req.adapter_policy.validation_profile,Some(ValidationProfile::NodeVite{..})),"tmpfs_bytes":if matches!(req.adapter_policy.validation_profile,Some(ValidationProfile::NodeVite{..})){2147483648u64}else{268435456u64},"cap_drop":["ALL"],"no_new_privileges":true,"dns":["127.0.0.1"],"env_names":["HOME","CODEX_HOME","PYTHONDONTWRITEBYTECODE","HTTPS_PROXY","HTTP_PROXY","ALL_PROXY"],"host_home_mounted":false,"docker_socket_mounted":false,"github_credentials_present":false,"cpus":req.adapter_policy.cpus,"memory_mb":req.adapter_policy.memory_mb}),
    )?;
    docker(req, create, None, 15, 65536).await?;
    docker(req, args(&["start", &name]), None, 15, 65536).await?;
    // This static shell runs inside the already-contained image, never on the host.
    docker(
        req,
        args(&[
            "exec",
            "-i",
            &name,
            "/bin/sh",
            "-c",
            "umask 077; mkdir -p /tmp/codex /tmp/home; cat > /tmp/codex/auth.json",
        ]),
        Some(&auth_bytes),
        15,
        65536,
    )
    .await?;
    let remaining = ((req.stage_deadline_ms - now_ms()).max(1) as u64).div_ceil(1000);
    write_new(
        &action_dir.join("model-intent.json"),
        &json!({"action_id":req.action_id,"at_ms":now_ms()}),
    )?;
    let output = docker(
        req,
        args(&["exec", "-i", &name, "/usr/local/bin/workboard-contained"]),
        Some(req_to_container(req)?.to_string().as_bytes()),
        remaining,
        65536,
    )
    .await?;
    if req.action == "plan" {
        let result: Value = serde_json::from_slice(&output)
            .map_err(|_| err("adapter_worker_result_schema_invalid"))?;
        if !result.as_object().is_some_and(|o| {
            o.len() == 4
                && ["schema_version", "completed", "session_id", "plan"]
                    .iter()
                    .all(|k| o.contains_key(*k))
        }) || result["schema_version"] != 1
            || result["completed"] != true
            || !result["session_id"].as_str().is_some_and(|s| {
                (8..=128).contains(&s.len())
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-:.".contains(&b))
            })
        {
            return Err(err("adapter_worker_result_schema_invalid"));
        }
        let draft: crate::planning::Draft = serde_json::from_value(result["plan"].clone())
            .map_err(|_| err("adapter_worker_result_schema_invalid"))?;
        draft.validate()?;
        if preserve_auth(req, &name, action_dir, Some(&auth_bytes)).await? {
            return Err(err("adapter_auth_refresh_quarantined"));
        }
        docker(req, args(&["rm", "--force", &name]), None, 15, 65536).await?;
        if !containers(req).await?.is_empty() {
            return Err(err("adapter_cleanup_unknown"));
        }
        let mut receipt = req.receipt();
        receipt.session_id = result["session_id"].as_str().map(String::from);
        receipt.role = Some("plan".into());
        receipt.plan = Some(draft);
        return Ok(receipt);
    }
    let result: Value =
        serde_json::from_slice(&output).map_err(|_| err("adapter_worker_result_schema_invalid"))?;
    if result.as_object().is_none_or(|o| {
        !(o.len() == 4
            || (o.len() == 5
                && (o.contains_key("findings") || o.contains_key("out_of_scope_bugs")))
            || (o.len() == 6 && o.contains_key("findings") && o.contains_key("out_of_scope_bugs")))
            || o.keys().any(|k| {
                ![
                    "schema_version",
                    "completed",
                    "passed",
                    "session_id",
                    "findings",
                    "out_of_scope_bugs",
                ]
                .contains(&k.as_str())
            })
    }) || result["schema_version"] != 1
        || !result["completed"].is_boolean()
        || !result["passed"].is_boolean()
        || !result["session_id"].as_str().is_some_and(|s| {
            (8..=128).contains(&s.len())
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-:.".contains(&b))
        })
    {
        return Err(err("adapter_worker_result_schema_invalid"));
    }
    let evidence_bytes = docker(
        req,
        args(&[
            "exec",
            &name,
            "/usr/local/bin/workboard-contained",
            "--export-evidence",
        ]),
        None,
        15,
        65536,
    )
    .await?;
    let evidence = validation_evidence(&evidence_bytes, req)?;
    // Keep only finite measurements before semantic acceptance. In particular, do
    // not retain an untrusted mismatching session string or the provider verdict.
    let test_exit_code = evidence["validation"]["test_exit_code"]
        .as_i64()
        .or_else(|| {
            evidence["commands"]
                .as_array()?
                .iter()
                .find(|v| v["kind"] == "test")?["exit_code"]
                .as_i64()
        });
    write_new(
        &action_dir.join("validation-status.json"),
        &json!({"schema_version":1,"result_completed":result["completed"],"result_passed":result["passed"],"session_matches":evidence["session_id"] == result["session_id"],"turn_completed":evidence["turn_completed"],"successful_tests":evidence["successful_tests"],"failed_tests":evidence["failed_tests"],"diff_reviews":evidence["diff_reviews"],"unittest":evidence.get("unittest"),"test_exit_code":test_exit_code,"profile_test_counts":evidence["validation"].get("tests")}),
    )?;
    if evidence["session_id"] != result["session_id"] {
        return Err(err("adapter_validation_session_mismatch"));
    }
    if evidence["turn_completed"] != true {
        return Err(err("adapter_validation_turn_incomplete"));
    }
    if result["passed"] == true
        && (evidence["successful_tests"].as_u64().unwrap_or(0) == 0
            || (req.action == "coder" && evidence["diff_reviews"].as_u64().unwrap_or(0) == 0))
    {
        return Err(err("adapter_validation_success_missing"));
    }
    let feedback: Option<Vec<crate::qa_feedback::Finding>> = result
        .get("findings")
        .map(|v| serde_json::from_value(v.clone()).map_err(|_| err("adapter_qa_feedback_invalid")))
        .transpose()?;
    if let Some(findings) = &feedback {
        crate::qa_feedback::findings(findings).map_err(|_| err("adapter_qa_feedback_invalid"))?;
    }
    let bugs: Option<Vec<crate::qa_feedback::OutOfScopeBug>> = result
        .get("out_of_scope_bugs")
        .map(|v| serde_json::from_value(v.clone()).map_err(|_| err("adapter_qa_feedback_invalid")))
        .transpose()?;
    if let Some(bugs) = &bugs {
        crate::qa_feedback::out_of_scope_bugs(bugs, req.base_sha.as_deref().unwrap_or(""))?;
    }
    if req.action != "qa" && bugs.as_ref().is_some_and(|b| !b.is_empty()) {
        return Err(err("adapter_qa_feedback_invalid"));
    }
    if req.action == "qa" {
        if result["completed"] != true {
            return Err(err("adapter_qa_incomplete"));
        }
        if (result["passed"] == true && feedback.as_ref().is_some_and(|v| !v.is_empty()))
            || (result["passed"] == false
                && bugs.as_ref().is_some_and(|v| !v.is_empty())
                && feedback.as_ref().is_none_or(|v| v.is_empty()))
        {
            return Err(err("adapter_qa_feedback_invalid"));
        }
        if result["passed"] == false
            && (feedback.as_ref().is_some_and(|v| !v.is_empty())
                || bugs.as_ref().is_some_and(|v| !v.is_empty()))
            && !negative_qa_complete(&evidence)
        {
            return Err(err("adapter_qa_validation_incomplete"));
        }
    }
    write_new(&action_dir.join("validation.json"), &evidence)?;
    if req.action == "coder" && result["completed"] == true && result["passed"] == true {
        let patch = docker(
            req,
            args(&[
                "exec",
                &name,
                "/usr/local/bin/workboard-contained",
                "--export-patch",
            ]),
            None,
            15,
            1048576,
        )
        .await?;
        if patch.is_empty() {
            return Err(err("adapter_patch_invalid"));
        }
        write_bytes(&action_dir.join("implementation.patch"), &patch)?;
    }
    if preserve_auth(req, &name, action_dir, Some(&auth_bytes)).await? {
        return Err(err("adapter_auth_refresh_quarantined"));
    }
    docker(req, args(&["rm", "--force", &name]), None, 15, 65536).await?;
    if !containers(req).await?.is_empty() {
        return Err(err("adapter_cleanup_unknown"));
    }
    let mut receipt = req.receipt();
    receipt.session_id = result["session_id"].as_str().map(String::from);
    receipt.role = Some(req.action.clone());
    if req.action == "coder" && req.task_snapshot["qa_policy"] == "skip" {
        receipt.passed = Some(result["completed"] == true && result["passed"] == true);
    }
    if req.action == "qa" {
        if profiles::node(&req.adapter_policy.validation_profile) {
            write_new(
                &action_dir.join("validation-binding.json"),
                &json!({"schema_version":1,"validated_head_sha":req.head_sha,"source_tree_sha256":evidence["validation"]["source_after_sha256"],"profile":req.adapter_policy.validation_profile}),
            )?;
        }
        receipt.reviewed_sha = req.head_sha.clone();
        receipt.passed = Some(result["completed"] == true && result["passed"] == true);
        if let Some(bugs) = bugs.filter(|b| !b.is_empty()) {
            receipt.out_of_scope_bugs = Some(bugs);
            receipt.qa_completed = Some(true);
        }
        if receipt.passed == Some(false) {
            receipt.outcome = "failed".into();
            if let Some(feedback) = feedback {
                receipt.findings = Some(feedback);
                receipt.qa_completed = Some(true);
            }
        }
        return Ok(receipt);
    }
    if result["completed"] != true || result["passed"] != true {
        receipt.outcome = "failed".into();
        return Ok(receipt);
    }
    let patch = std::fs::read(action_dir.join("implementation.patch"))
        .map_err(|_| err("adapter_patch_invalid"))?;
    // git apply rejects traversal and protected Git paths; numstat adds a case-insensitive .git guard for macOS.
    let paths = git(req, &["apply", "--numstat", "-z", "-"], Some(&patch)).await?;
    for record in paths.split('\0').filter(|s| !s.is_empty()) {
        let path = record
            .splitn(3, '\t')
            .nth(2)
            .ok_or_else(|| err("adapter_patch_invalid"))?;
        if path.is_empty()
            || Path::new(path).is_absolute()
            || path
                .split('/')
                .any(|p| p == ".." || p.eq_ignore_ascii_case(".git"))
        {
            return Err(err("adapter_patch_invalid"));
        }
    }
    // Preparing read-only container visibility can change inode timestamps.
    // Refresh Git's stat cache before --index verifies identical worktree bytes.
    git(req, &["update-index", "--refresh"], None).await?;
    git(req, &["apply", "--check", "--index", "-"], Some(&patch)).await?;
    git(req, &["apply", "--index", "-"], Some(&patch)).await?;
    git(
        req,
        &[
            "-c",
            "user.name=Workboard Coder",
            "-c",
            "user.email=workboard-coder@users.noreply.github.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            &format!("Implement Workboard run {}", req.run_id),
        ],
        None,
    )
    .await?;
    super::publisher::readable_source(&req.workspace)?;
    git(req, &["update-index", "--refresh"], None).await?;
    let head = git(req, &["rev-parse", "HEAD"], None).await?;
    receipt.head_sha = Some(head.trim().into());
    if profiles::node(&req.adapter_policy.validation_profile) {
        let tree = profiles::source_tree(&req.workspace)?.digest();
        if evidence["validation"]["source_after_sha256"] != tree {
            return Err(err("adapter_profile_drift"));
        }
        write_new(
            &action_dir.join("validation-binding.json"),
            &json!({"schema_version":1,"validated_head_sha":receipt.head_sha,"source_tree_sha256":tree,"profile":req.adapter_policy.validation_profile}),
        )?;
    }
    std::fs::remove_file(action_dir.join("implementation.patch"))
        .map_err(|_| err("adapter_patch_invalid"))?;
    Ok(receipt)
}
pub(super) fn req_to_container(req: &Request) -> Result<Value> {
    let mut value = json!({"schema_version":1,"role":req.action,"repository":req.repository,"task_snapshot":req.task_snapshot,"base_sha":req.base_sha,"head_sha":req.head_sha,"model":req.adapter_policy.model});
    if let Some(agent) = req
        .adapter_policy
        .agent_config
        .as_ref()
        .and_then(|c| c.agent(&req.action))
    {
        value["agent_context"] =
            serde_json::to_value(agent).map_err(|_| err("adapter_request_invalid"))?;
        if !agent.model.is_empty() {
            value["model"] = json!(agent.model);
        }
    }
    if let Some(repair) = &req.repair_context {
        value["repair_context"] =
            serde_json::to_value(repair).map_err(|_| err("adapter_request_invalid"))?;
    }
    if let Some(profile) = &req.adapter_policy.validation_profile {
        value["validation_profile"] =
            serde_json::to_value(profile).map_err(|_| err("adapter_profile_invalid"))?;
    }
    Ok(value)
}

async fn cancel(req: &Request) -> Result<()> {
    let mut refreshed = false;
    for id in containers(req).await? {
        let inspected: Value =
            serde_json::from_slice(&docker(req, args(&["inspect", &id]), None, 10, 65536).await?)
                .map_err(|_| err("adapter_cleanup_unknown"))?;
        let c = &inspected[0];
        let action = c["Config"]["Labels"]["io.workboard.action"]
            .as_str()
            .ok_or_else(|| err("adapter_cleanup_unknown"))?;
        if Uuid::parse_str(action).is_err()
            || c["Image"] != req.adapter_policy.worker_image
            || c["Config"]["Labels"]["io.workboard.run"] != req.run_id
            || c["Config"]["Labels"]["io.workboard.approval"] != req.approval_sha256
        {
            return Err(err("adapter_cleanup_unknown"));
        }
        let recorded = read_value(
            &req.adapter_policy
                .state_root
                .join(&req.run_id)
                .join(action)
                .join("intent.json"),
        )?;
        let expected = format!("workboard-{}-{action}", req.run_id);
        if recorded["container"] != expected || c["Name"] != format!("/{expected}") {
            return Err(err("adapter_cleanup_unknown"));
        }
        let old_action = req.adapter_policy.state_root.join(&req.run_id).join(action);
        if old_action.join("model-intent.json").exists() {
            refreshed |= preserve_auth(req, &id, &old_action, None).await?;
        }
        docker(req, args(&["rm", "--force", &id]), None, 15, 65536).await?;
    }
    if !containers(req).await?.is_empty() {
        return Err(err("adapter_cleanup_unknown"));
    }
    if refreshed {
        return Err(err("adapter_auth_refresh_quarantined"));
    }
    Ok(())
}

async fn preserve_auth(
    req: &Request,
    name: &str,
    action_dir: &Path,
    initial: Option<&[u8]>,
) -> Result<bool> {
    let stopped: Value = serde_json::from_slice(
        &docker(
            req,
            args(&[
                "exec",
                name,
                "/usr/local/bin/workboard-contained",
                "--stop-worker",
            ]),
            None,
            10,
            65536,
        )
        .await?,
    )
    .map_err(|_| err("adapter_auth_refresh_unknown"))?;
    if stopped["stopped"] != true {
        return Err(err("adapter_auth_refresh_unknown"));
    }
    let current = docker(
        req,
        args(&[
            "exec",
            name,
            "/usr/local/bin/workboard-contained",
            "--export-auth",
        ]),
        None,
        10,
        131072,
    )
    .await?;
    let reference = req.adapter_policy.auth_home.join("auth.json");
    let canonical;
    let seeded = if let Some(bytes) = initial {
        bytes
    } else {
        let recorded = read_value(&action_dir.join("auth-reference.json"))?;
        if auth_metadata(&reference)? != recorded {
            return Err(err("adapter_auth_reference_changed"));
        }
        let (bytes, metadata) = read_auth(&reference)?;
        canonical = bytes;
        if metadata != recorded {
            return Err(err("adapter_auth_reference_changed"));
        }
        canonical.as_slice()
    };
    if current == seeded {
        return Ok(false);
    }
    if !serde_json::from_slice::<Value>(&current).is_ok_and(|v| v.is_object()) {
        return Err(err("adapter_auth_refresh_unknown"));
    }
    let recovery = action_dir.join("auth-recovery.json");
    if recovery.exists() {
        if std::fs::read(&recovery).map_err(|_| err("adapter_auth_refresh_unknown"))? != current {
            return Err(err("adapter_auth_refresh_unknown"));
        }
        return Ok(true);
    }
    use std::io::Write;
    let temp = action_dir.join("auth-recovery.pending");
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
    let mut file = options
        .open(&temp)
        .map_err(|_| err("adapter_auth_refresh_unknown"))?;
    file.write_all(&current)
        .and_then(|_| file.sync_all())
        .map_err(|_| err("adapter_auth_refresh_unknown"))?;
    std::fs::rename(temp, recovery).map_err(|_| err("adapter_auth_refresh_unknown"))?;
    std::fs::File::open(action_dir)
        .and_then(|f| f.sync_all())
        .map_err(|_| err("adapter_auth_refresh_unknown"))?;
    Ok(true)
}

fn auth_metadata(path: &Path) -> Result<Value> {
    metadata_value(
        &std::fs::symlink_metadata(path).map_err(|_| err("adapter_auth_reference_invalid"))?,
    )
}
fn metadata_value(m: &std::fs::Metadata) -> Result<Value> {
    use std::os::unix::fs::MetadataExt;
    if !m.is_file() || m.permissions().mode() & 0o077 != 0 || m.len() > 131072 {
        return Err(err("adapter_auth_reference_invalid"));
    }
    Ok(
        json!({"device":m.dev(),"inode":m.ino(),"size":m.len(),"mtime":m.mtime(),"mtime_ns":m.mtime_nsec()}),
    )
}

// Bind metadata to the exact read-only file descriptor used for the seed, and
// reject reference replacement during the read. No normal-path auth copy/digest.
fn read_auth(path: &Path) -> Result<(Vec<u8>, Value)> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let expected = auth_metadata(path)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| err("adapter_auth_unavailable"))?;
    let initial = metadata_value(
        &file
            .metadata()
            .map_err(|_| err("adapter_auth_unavailable"))?,
    )?;
    if expected != initial {
        return Err(err("adapter_auth_reference_changed"));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(131073)
        .read_to_end(&mut bytes)
        .map_err(|_| err("adapter_auth_unavailable"))?;
    if bytes.len() > 131072
        || metadata_value(
            &file
                .metadata()
                .map_err(|_| err("adapter_auth_unavailable"))?,
        )? != initial
        || auth_metadata(path)? != initial
    {
        return Err(err("adapter_auth_reference_changed"));
    }
    Ok((bytes, initial))
}

// Only the trusted helper's fixed projection may enter retained validation state.
fn validation_evidence(bytes: &[u8], req: &Request) -> Result<Value> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| err("adapter_validation_evidence_invalid"))?;
    if let Some(profile) = req
        .adapter_policy
        .validation_profile
        .as_ref()
        .filter(|p| p.node())
    {
        profiles::evidence(
            &value,
            profile,
            req.head_sha.as_deref().unwrap_or(""),
            &profiles::source_tree(&req.workspace)?.digest(),
            req.action == "qa",
        )?;
        return Ok(value);
    }
    let fields = [
        "schema_version",
        "session_id",
        "turn_completed",
        "successful_tests",
        "failed_tests",
        "diff_reviews",
        "commands",
        "unittest",
    ];
    if !value
        .as_object()
        .is_some_and(|o| o.len() == fields.len() && fields.iter().all(|f| o.contains_key(*f)))
        || value["schema_version"] != 1
        || !value["turn_completed"].is_boolean()
        || ["successful_tests", "failed_tests", "diff_reviews"]
            .iter()
            .any(|k| value[k].as_u64().is_none_or(|n| n > 1000000))
    {
        return Err(err("adapter_validation_evidence_invalid"));
    }
    let counts = &value["unittest"];
    let count_fields = ["run", "executed", "skipped", "failures", "errors"];
    if !counts.as_object().is_some_and(|o| {
        o.len() == count_fields.len() && count_fields.iter().all(|f| o.contains_key(*f))
    }) || count_fields
        .iter()
        .any(|k| counts[k].as_u64().is_none_or(|n| n > 1000000))
    {
        return Err(err("adapter_validation_evidence_invalid"));
    }
    let commands = value["commands"]
        .as_array()
        .filter(|a| a.len() <= 258)
        .ok_or_else(|| err("adapter_validation_evidence_invalid"))?;
    for command in commands {
        let fields = match command["kind"].as_str() {
            Some("model_command") => vec!["kind", "exit_code", "completed"],
            Some("test") if command["command"] == "python3 -m unittest -v" => {
                vec!["kind", "command", "exit_code", "completed"]
            }
            Some("diff_review") if command["command"] == "git diff --check" => {
                vec!["kind", "command", "exit_code", "completed"]
            }
            _ => return Err(err("adapter_validation_evidence_invalid")),
        };
        if !command
            .as_object()
            .is_some_and(|o| o.len() == fields.len() && fields.iter().all(|f| o.contains_key(*f)))
            || !command["completed"].is_boolean()
            || (!command["exit_code"].is_null()
                && command["exit_code"]
                    .as_i64()
                    .is_none_or(|v| i32::try_from(v).is_err()))
        {
            return Err(err("adapter_validation_evidence_invalid"));
        }
    }
    Ok(value)
}

// Native Vite dependencies execute only inside this existing nonroot container.
// Logical tmpfs capacity does not change the signed cgroup memory limit.
fn temporary_filesystems(profile: Option<&ValidationProfile>) -> (&'static str, &'static str) {
    if matches!(profile, Some(ValidationProfile::NodeVite { .. })) {
        (
            "/tmp:rw,noexec,nosuid,nodev,size=2147483648,mode=1777",
            "/workspace:rw,exec,nosuid,nodev,size=536870912,mode=0700,uid=65532,gid=65532",
        )
    } else {
        (
            "/tmp:rw,noexec,nosuid,nodev,size=268435456,mode=1777",
            "/workspace:rw,noexec,nosuid,nodev,size=536870912,mode=0700,uid=65532,gid=65532",
        )
    }
}
#[cfg(test)]
mod resource_tests {
    use super::*;
    #[test]
    fn native_dependency_execution_and_large_logical_tmp_are_vite_only() {
        let vite = ValidationProfile::NodeVite {
            manifest_sha256: "a".repeat(64),
            lock_sha256: "b".repeat(64),
            cache_sha256: "c".repeat(64),
        };
        let (tmp, workspace) = temporary_filesystems(Some(&vite));
        assert!(tmp.contains("size=2147483648") && tmp.contains("noexec,nosuid,nodev"));
        assert!(workspace.contains(",exec,") && workspace.contains("uid=65532,gid=65532"));
        let builtin = ValidationProfile::NodeBuiltin {
            manifest_sha256: "a".repeat(64),
            build_sha256: "b".repeat(64),
        };
        let npm = ValidationProfile::Node {
            manifest_sha256: "a".repeat(64),
            lock_sha256: "b".repeat(64),
            check_sha256: "c".repeat(64),
            cache_sha256: "d".repeat(64),
        };
        for profile in [
            None,
            Some(&builtin),
            Some(&npm),
            Some(&ValidationProfile::Python {}),
        ] {
            let (tmp, workspace) = temporary_filesystems(profile);
            assert!(tmp.contains("size=268435456"));
            assert!(workspace.contains(",noexec,") && workspace.contains("size=536870912"));
        }
    }
}

fn negative_qa_complete(e: &Value) -> bool {
    if e["turn_completed"] != true {
        return false;
    }
    if let Some(n) = e["unittest"]["executed"].as_u64() {
        let commands = e["commands"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["kind"] == "test" && c["command"] == "python3 -m unittest -v")
            .collect::<Vec<_>>();
        if commands.len() != 1 || commands[0]["completed"] != true {
            return false;
        }
        let Some(exit) = commands[0]["exit_code"].as_i64().filter(|n| *n >= 0) else {
            return false;
        };
        let failures = e["unittest"]["failures"].as_u64().unwrap_or(0)
            + e["unittest"]["errors"].as_u64().unwrap_or(0);
        return n > 0
            && e["unittest"]["run"].as_u64()
                == Some(n + e["unittest"]["skipped"].as_u64().unwrap_or(0))
            && ((exit == 0 && failures == 0) || (exit > 0 && failures > 0));
    }
    let v = &e["validation"];
    let counts = &v["tests"];
    let executed = counts["pass"].as_u64().unwrap_or(0) + counts["fail"].as_u64().unwrap_or(0);
    executed > 0
        && counts["cancelled"] == 0
        && v["install_exit_code"].as_i64().is_none_or(|n| n == 0)
        && v["dependency_import_exit_code"]
            .as_i64()
            .is_none_or(|n| n == 0)
        && [
            "test_exit_code",
            "syntax_check_exit_code",
            "typecheck_exit_code",
            "vite_build_exit_code",
            "review_build_exit_code",
        ]
        .iter()
        .all(|k| v[k].is_null() || v[k].as_i64().is_some_and(|n| n >= 0))
        && (v["test_exit_code"] == 0 || counts["fail"].as_u64().unwrap_or(0) > 0)
}

#[cfg(test)]
mod negative_qa_tests {
    use super::*;
    #[test]
    fn completed_test_failures_are_known_negatives_but_install_signal_zero_or_cancel_are_not() {
        let python = json!({"turn_completed":true,"commands":[{"kind":"test","command":"python3 -m unittest -v","completed":true,"exit_code":1}],"unittest":{"run":2,"executed":2,"skipped":0,"failures":1,"errors":0}});
        assert!(negative_qa_complete(&python));
        for (key, value) in [
            ("completed", json!(false)),
            ("exit_code", Value::Null),
            ("exit_code", json!(-1)),
            ("exit_code", json!(0)),
        ] {
            let mut bad = python.clone();
            bad["commands"][0][key] = value;
            assert!(!negative_qa_complete(&bad));
        }
        let v = json!({"turn_completed":true,"validation":{"install_exit_code":0,"typecheck_exit_code":1,"vite_build_exit_code":1,"test_exit_code":1,"tests":{"pass":1,"fail":1,"cancelled":0}}});
        assert!(negative_qa_complete(&v));
        for (pointer, changed) in [
            ("/turn_completed", json!(false)),
            ("/validation/install_exit_code", json!(1)),
            ("/validation/test_exit_code", json!(-1)),
            ("/validation/tests/cancelled", json!(1)),
        ] {
            let mut bad = v.clone();
            *bad.pointer_mut(pointer).unwrap() = changed;
            assert!(!negative_qa_complete(&bad));
        }
        let mut unit = json!({"turn_completed":true,"validation":{"syntax_check_exit_code":-1,"test_exit_code":1,"tests":{"pass":1,"fail":1,"cancelled":0}}});
        assert!(!negative_qa_complete(&unit));
        unit["validation"]["syntax_check_exit_code"] = json!(1);
        assert!(negative_qa_complete(&unit));
        let mut zero = v;
        zero["validation"]["tests"] = json!({"pass":0,"fail":0,"cancelled":0});
        assert!(!negative_qa_complete(&zero));
    }
}
