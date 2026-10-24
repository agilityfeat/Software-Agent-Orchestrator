//! Native adapter protocol with a synthetic CLI, real local Git, and no provider/network.
#![cfg(unix)]
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use uuid::Uuid;
fn private(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}
fn git(root: &Path, args: &[&str]) -> String {
    let o = Command::new("/usr/bin/git")
        .args(["-C", root.to_str().unwrap()])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap().trim().into()
}
fn program(path: &Path) -> Value {
    json!({"executable":path,"sha256":format!("{:x}",Sha256::digest(std::fs::read(path).unwrap())),"credential_env_names":[]})
}
struct Harness {
    _root: tempfile::TempDir,
    dir: PathBuf,
    req: Value,
}
impl Harness {
    fn new(mode: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().canonicalize().unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        for n in ["state", "auth", "projects"] {
            std::fs::create_dir(dir.join(n)).unwrap();
            std::fs::set_permissions(dir.join(n), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        private(
            &dir.join("auth/auth.json"),
            br#"{"tokens":{"account_id":"synthetic-account","access_token":"synthetic-not-real"}}"#,
        );
        let cli = dir.join("codex");
        let script = include_str!("fixtures/native.py").replace("MODE_VALUE", &format!("{mode:?}"));
        std::fs::write(&cli, script).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        let run = Uuid::new_v4().to_string();
        let workspace = dir.join(&run);
        std::fs::create_dir(&workspace).unwrap();
        git(&workspace, &["init", "--quiet"]);
        git(&workspace, &["config", "user.name", "Synthetic"]);
        git(
            &workspace,
            &["config", "user.email", "synthetic@example.invalid"],
        );
        std::fs::write(workspace.join("code.py"), "value = 1\n").unwrap();
        std::fs::write(workspace.join(".gitignore"), ".venv/\n").unwrap();
        std::fs::create_dir(workspace.join(".codex")).unwrap();
        std::fs::write(
            workspace.join(".codex/config.toml"),
            "# tracked protected metadata\n",
        )
        .unwrap();
        git(&workspace, &["add", "."]);
        git(&workspace, &["commit", "--quiet", "-m", "base"]);
        let head = git(&workspace, &["rev-parse", "HEAD"]);
        let native = json!({"codex":program(&cli),"projects_root":dir.join("projects"),"catalog_path":dir.join("catalog.json"),"path":"/usr/bin:/bin","read_roots":[],"agents":[{"id":"coder","label":"Coder","instructions":"Synthetic"},{"id":"qa","label":"QA","instructions":"Independent"}],"default_coder":"coder","qa_agent":"qa"});
        let gp = program(Path::new("/usr/bin/git"));
        let ap = json!({"repository":"fixture/public","state_root":dir.join("state"),"auth_home":dir.join("auth"),"docker":gp,"git":gp,"gh":gp,"docker_host":"unused","worker_image":"unused","worker_network":"unused","proxy_container":"unused","proxy_image":"unused","proxy_ip":"unused","proxy_port":0,"proxy_policy_sha256":"unused","proxy_allowed_hosts":[],"model":null,"cpus":1,"memory_mb":256,"native_agent":native});
        let req = json!({"schema_version":1,"action":"coder","action_id":Uuid::new_v4().to_string(),"run_id":run,"repository":"fixture/public","task_snapshot":{"repository":"fixture/public","workflow":"engineering","merge_policy":"auto","title":"Synthetic","coding_agent_id":"coder"},"deadline_ms":chrono::Utc::now().timestamp_millis()+60000,"stage_deadline_ms":chrono::Utc::now().timestamp_millis()+30000,"workspace":workspace,"base_sha":head,"head_sha":head,"base_ref":"main","qa_receipt":null,"merge_policy":"auto","observed_action_id":null,"approval_sha256":"6".repeat(64),"adapter_policy":ap});
        Self {
            _root: root,
            dir,
            req,
        }
    }
    fn invoke(&self) -> std::process::Output {
        self.invoke_role(false)
    }
    fn invoke_role(&self, publisher: bool) -> std::process::Output {
        let mut c = Command::new(if publisher {
            env!("CARGO_BIN_EXE_workboard-publisher")
        } else {
            env!("CARGO_BIN_EXE_workboard-worker")
        })
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GH_TOKEN", "synthetic-token-never-real")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
        c.stdin
            .take()
            .unwrap()
            .write_all(self.req.to_string().as_bytes())
            .unwrap();
        c.wait_with_output().unwrap()
    }
    fn prepare_programs(&mut self) -> PathBuf {
        let remote = self.dir.join("remote");
        std::fs::rename(self.req["workspace"].as_str().unwrap(), &remote).unwrap();
        git(&remote, &["branch", "-M", "main"]);
        let wrap = self.dir.join("git-wrapper");
        let script = format!(
            r#"#!/usr/bin/python3
import sys,subprocess
args=sys.argv[1:]
origin=next((a for a in args if a.startswith('https://github.com/')),None)
if 'clone' in args:
 args=[{remote:?} if a==origin else ('protocol.file.allow=always' if a=='protocol.file.allow=never' else a) for a in args]
 out=subprocess.run(['/usr/bin/git','-c','protocol.file.allow=always',*args],stdout=subprocess.PIPE,stderr=subprocess.PIPE)
 if out.returncode==0:subprocess.check_call(['/usr/bin/git','-C',args[-1],'remote','set-url','origin',origin])
 sys.stdout.buffer.write(out.stdout);sys.stderr.buffer.write(out.stderr);sys.exit(out.returncode)
result=subprocess.run(['/usr/bin/git',*args],input=sys.stdin.buffer.read());sys.exit(result.returncode)
"#,
            remote = remote.to_str().unwrap()
        );
        std::fs::write(&wrap, script).unwrap();
        std::fs::set_permissions(&wrap, std::fs::Permissions::from_mode(0o700)).unwrap();
        let gh = self.dir.join("gh");
        std::fs::write(&gh,r#"#!/usr/bin/python3
import sys,json
repo=next(a for a in sys.argv if a.startswith('repos/'))[6:]
print(json.dumps({'id':99 if repo=='fixture/public' else 100,'full_name':repo,'private':False,'permissions':{'push':True},'default_branch':'main','archived':False}))
"#).unwrap();
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o700)).unwrap();
        self.req["adapter_policy"]["git"] = program(&wrap);
        self.req["adapter_policy"]["gh"] = program(&gh);
        self.req["action"] = json!("prepare");
        self.req["base_sha"] = Value::Null;
        self.req["head_sha"] = Value::Null;
        remote
    }
    fn fresh_run(&mut self, repository: &str) {
        let id = Uuid::new_v4().to_string();
        self.req["run_id"] = json!(id);
        self.req["action_id"] = json!(Uuid::new_v4().to_string());
        self.req["workspace"] = json!(self.dir.join(id));
        self.req["repository"] = json!(repository);
        self.req["task_snapshot"]["repository"] = json!(repository);
        self.req["adapter_policy"]["repository"] = json!(repository);
    }
    fn action(&self) -> PathBuf {
        self.dir
            .join("state")
            .join(self.req["run_id"].as_str().unwrap())
            .join(self.req["action_id"].as_str().unwrap())
    }
    fn ok(&self) -> Value {
        let o = self.invoke();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        serde_json::from_slice(&o.stdout).unwrap()
    }
}
fn planner(mode: &str) -> Harness {
    let mut h = Harness::new(mode);
    std::fs::remove_dir_all(h.req["workspace"].as_str().unwrap()).unwrap();
    h.req["action"] = json!("plan");
    h.req["base_sha"] = Value::Null;
    h.req["head_sha"] = Value::Null;
    h.req["adapter_policy"]["native_agent"]["native_planning"] = json!(true);
    h.req["adapter_policy"]["agent_config"] = json!({"revision":1,"main":{"id":"main","label":"Synthetic Main","role":"main","instructions":"Frozen synthetic Main instructions","model":"","enabled":true}});
    h.req["task_snapshot"]["goal"] = json!("Synthetic goal only");
    h
}
#[test]
fn native_main_plans_without_docker_source_or_tools_and_replays_only_receipt() {
    let h = planner("plan");
    let receipt = h.ok();
    assert_eq!(receipt["role"], "plan");
    assert_eq!(receipt["cleanup_complete"], true);
    assert_eq!(receipt["plan"]["tasks"].as_array().unwrap().len(), 1);
    assert!(receipt["head_sha"].is_null());
    assert!(!h.action().join("codex-home").exists());
    assert_eq!(h.ok(), receipt);
    assert!(!h.action().join("container-policy.json").exists());
}
#[test]
fn native_main_rejects_nonempty_workspace_before_auth_or_spawn() {
    let h = planner("plan");
    let workspace = Path::new(h.req["workspace"].as_str().unwrap());
    std::fs::create_dir(workspace).unwrap();
    std::fs::write(workspace.join("sentinel"), b"untouched").unwrap();
    assert!(!h.invoke().status.success());
    assert!(!h.action().join("spawn-intent.json").exists());
    assert!(!h.action().join("auth-reference.json").exists());
    assert!(h.action().join("cleanup.json").exists());
    assert_eq!(
        std::fs::read(workspace.join("sentinel")).unwrap(),
        b"untouched"
    );
}
#[test]
fn native_main_tool_malformed_incomplete_or_deadline_never_proposes_and_cleans_up() {
    for mode in [
        "plan_tool",
        "plan_malformed",
        "plan_incomplete",
        "plan_sleep",
    ] {
        let mut h = planner(mode);
        if mode == "plan_sleep" {
            h.req["stage_deadline_ms"] = json!(chrono::Utc::now().timestamp_millis() + 5000);
        }
        assert!(!h.invoke().status.success(), "{mode}");
        assert!(!h.action().join("native-receipt.json").exists());
        assert!(h.action().join("cleanup.json").exists());
        assert!(!h.action().join("codex-home").exists());
        if mode == "plan_sleep" {
            let pid = std::fs::read_to_string(h.action().join("cache/child.pid")).unwrap();
            assert!(
                !Command::new("/bin/ps")
                    .args(["-p", pid.trim(), "-o", "pid="])
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
    }
}
#[test]
fn native_actual_transport_coder_qa_and_large_task() {
    let mut h = Harness::new("success");
    h.req["task_snapshot"]["objective"] = json!("x".repeat(20000));
    let before = h.req["head_sha"].clone();
    let r = h.ok();
    assert_ne!(r["head_sha"], before);
    assert_eq!(r["passed"], true);
    assert_eq!(r["cleanup_complete"], true);
    assert!(h.action().join("cleanup.json").exists());
    assert!(!h.action().join("codex-home").exists());
    let replay = h.ok();
    assert_eq!(r, replay);
    h.req["action"] = json!("qa");
    h.req["action_id"] = json!(Uuid::new_v4().to_string());
    h.req["head_sha"] = r["head_sha"].clone();
    let qa = h.ok();
    assert_eq!(qa["reviewed_sha"], r["head_sha"]);
    assert_eq!(qa["qa_completed"], true);
    assert_ne!(qa["session_id"], r["session_id"]);
    assert_eq!(
        git(
            Path::new(h.req["workspace"].as_str().unwrap()),
            &["status", "--porcelain"]
        ),
        ""
    );
}
#[test]
fn native_literal_shell_wrapper_transport_retains_actual_command_ids() {
    let h = Harness::new("wrapped");
    let receipt = h.ok();
    assert_eq!(receipt["passed"], true);
    assert_eq!(receipt["cleanup_complete"], true);
    let audit: Value =
        serde_json::from_slice(&std::fs::read(h.action().join("native-validation.json")).unwrap())
            .unwrap();
    assert_eq!(audit["reason"], "accepted");
    let validation: Value =
        serde_json::from_slice(&std::fs::read(h.action().join("validation.json")).unwrap())
            .unwrap();
    assert_eq!(
        validation["validation"]["checks"][0]["command_id"],
        "item_0"
    );
    assert!(!h.action().join("codex-home").exists());
}
#[test]
fn native_rejects_invented_checks_and_preserves_typed_failure() {
    for mode in ["echo", "mismatch", "id_mismatch", "exit", "blocked"] {
        let h = Harness::new(mode);
        let o = h.invoke();
        assert!(!o.status.success(), "{mode}");
        assert!(h.action().join("cleanup.json").exists(), "{mode}");
        assert!(!h.action().join("native-receipt.json").exists());
        if mode != "exit" {
            let audit: Value = serde_json::from_slice(
                &std::fs::read(h.action().join("native-validation.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(audit["session_started"], true);
            assert_eq!(audit["turn_completed"], true);
            assert_eq!(audit["declared_check_count"], 2);
            assert_eq!(audit["observed_command_count"], 2);
            let reason = match mode {
                "echo" => "command_incomplete_or_trivial",
                "mismatch" => "command_not_observed",
                "id_mismatch" => "model_command_id_not_null",
                "blocked" => "required_validation_missing_or_failed",
                _ => unreachable!(),
            };
            assert_eq!(audit["reason"], reason);
            let f: Value =
                serde_json::from_slice(&std::fs::read(h.action().join("failure.json")).unwrap())
                    .unwrap();
            assert_eq!(f["session_started"], true);
            assert_eq!(f["stage"], "validation");
            assert!(!audit.to_string().contains("Synthetic actual validation"));
            assert!(!audit.to_string().contains("invented test"));
        }
        if mode == "exit" {
            let f: Value =
                serde_json::from_slice(&std::fs::read(h.action().join("failure.json")).unwrap())
                    .unwrap();
            assert_eq!(f["stage"], "codex");
            assert_eq!(f["exit_code"], 7);
        }
    }
}
#[test]
fn native_deadline_and_broken_pipe_reap_owned_children() {
    for mode in ["sleep", "broken_pipe"] {
        let mut h = Harness::new(mode);
        h.req["stage_deadline_ms"] = json!(chrono::Utc::now().timestamp_millis() + 5000);
        let o = h.invoke();
        assert!(!o.status.success());
        assert!(
            h.action().join("cleanup.json").exists(),
            "{mode}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        let pid = std::fs::read_to_string(h.action().join("cache/child.pid")).unwrap();
        assert!(
            !Command::new("/bin/ps")
                .args(["-p", pid.trim(), "-o", "pid="])
                .output()
                .unwrap()
                .status
                .success(),
            "owned child survived {mode}"
        );
    }
}
#[test]
fn native_known_no_spawn_failure_can_be_cancelled() {
    let mut h = Harness::new("success");
    std::fs::remove_file(h.dir.join("auth/auth.json")).unwrap();
    assert!(!h.invoke().status.success());
    assert!(h.action().join("cleanup.json").exists());
    h.req["action"] = json!("cancel");
    h.req["action_id"] = json!(Uuid::new_v4().to_string());
    assert_eq!(h.ok()["cleanup_complete"], true);
}

#[test]
fn native_parallel_cold_catalog_and_recovery_preserve_known_checkout() {
    let mut h = Harness::new("success");
    let remote = h.prepare_programs();
    // A verified known root can be outside projects_root and have a dirty/local older HEAD.
    git(
        &remote,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/fixture/public.git",
        ],
    );
    let baseline = git(&remote, &["rev-parse", "HEAD"]);
    std::fs::write(remote.join("code.py"), "canonical dirty remains\n").unwrap();
    private(&h.dir.join("catalog.json"),json!({"schema_version":1,"workspaces":[{"repository":"fixture/public","repository_id":99,"path":remote,"known_by":["coder"],"environment_paths":[".venv"]}]}).to_string().as_bytes());
    let first = h.invoke_role(true);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(git(&remote, &["rev-parse", "HEAD"]), baseline);
    assert_eq!(
        std::fs::read_to_string(remote.join("code.py")).unwrap(),
        "canonical dirty remains\n"
    );
    // Missing catalog and concurrent cold repository preparations merge rather than starve each other.
    std::fs::remove_file(h.dir.join("catalog.json")).unwrap();
    h.fresh_run("fixture/public");
    let mut second = h.req.clone();
    let id = Uuid::new_v4().to_string();
    second["run_id"] = json!(id);
    second["action_id"] = json!(Uuid::new_v4().to_string());
    second["workspace"] = json!(h.dir.join(id));
    second["repository"] = json!("fixture/second");
    second["task_snapshot"]["repository"] = json!("fixture/second");
    second["adapter_policy"]["repository"] = json!("fixture/second");
    std::thread::scope(|scope| {
        let b = scope.spawn(|| {
            let mut c = Command::new(env!("CARGO_BIN_EXE_workboard-publisher"))
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("GH_TOKEN", "synthetic-token")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            c.stdin
                .take()
                .unwrap()
                .write_all(second.to_string().as_bytes())
                .unwrap();
            c.wait_with_output().unwrap()
        });
        let a = h.invoke_role(true);
        assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
        let b = b.join().unwrap();
        assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    });
    let catalog: Value =
        serde_json::from_slice(&std::fs::read(h.dir.join("catalog.json")).unwrap()).unwrap();
    assert_eq!(catalog["workspaces"].as_array().unwrap().len(), 2);
    // Durable managed marker recovers a registration lost after clone without deleting the existing checkout.
    std::fs::remove_file(h.dir.join("catalog.json")).unwrap();
    h.fresh_run("fixture/public");
    assert!(h.invoke_role(true).status.success());
}

#[test]
fn native_auth_refresh_same_account_cas_and_foreign_account_quarantine() {
    let h = Harness::new("refresh");
    h.ok();
    let source: Value =
        serde_json::from_slice(&std::fs::read(h.dir.join("auth/auth.json")).unwrap()).unwrap();
    assert_eq!(source["tokens"]["access_token"], "synthetic-refreshed-only");
    assert!(!h.action().join("auth-recovery.json").exists());
    for mode in ["foreign_account", "auth_drift"] {
        let h = Harness::new(mode);
        assert!(!h.invoke().status.success());
        assert!(h.action().join("auth-recovery.json").exists());
        assert!(!h.action().join("cleanup.json").exists());
        let source: Value =
            serde_json::from_slice(&std::fs::read(h.dir.join("auth/auth.json")).unwrap()).unwrap();
        assert_eq!(source["tokens"]["account_id"], "synthetic-account");
        assert_eq!(
            source["tokens"]["access_token"],
            if mode == "auth_drift" {
                "synthetic-newer-only"
            } else {
                "synthetic-not-real"
            }
        );
    }
}

#[test]
fn native_supervisor_owns_deadline_after_outer_driver_is_killed_without_replay() {
    let mut h = Harness::new("sleep");
    h.req["stage_deadline_ms"] = json!(chrono::Utc::now().timestamp_millis() + 6000);
    let mut c = Command::new(env!("CARGO_BIN_EXE_workboard-worker"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    c.stdin
        .take()
        .unwrap()
        .write_all(h.req.to_string().as_bytes())
        .unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while !h.action().join("cache/child.pid").exists() {
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    c.kill().unwrap();
    c.wait().unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(8);
    while !h.action().join("cleanup.json").exists() {
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!h.invoke().status.success()); // uncertain outer action never replays a launch
    assert!(!h.action().join("receipt.json").exists());
    h.req["action"] = json!("cancel");
    h.req["action_id"] = json!(Uuid::new_v4().to_string());
    assert_eq!(h.ok()["cleanup_complete"], true);
}
#[test]
fn native_qa_source_mutation_is_rejected_without_changing_implementation() {
    let mut h = Harness::new("qa_mutate");
    h.req["action"] = json!("qa");
    assert!(!h.invoke().status.success());
    assert!(h.action().join("cleanup.json").exists());
    assert_eq!(
        std::fs::read_to_string(Path::new(h.req["workspace"].as_str().unwrap()).join("code.py"))
            .unwrap(),
        "value = 1\n"
    );
}

#[test]
fn native_catalog_rejects_nested_root_and_recovers_a_moved_known_root() {
    for moved in [false, true] {
        let mut h = Harness::new("success");
        let remote = h.prepare_programs();
        git(
            &remote,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/fixture/public.git",
            ],
        );
        let hint = if moved {
            h.dir.join("deleted-known-root")
        } else {
            let path = remote.join("nested");
            std::fs::create_dir(&path).unwrap();
            path
        };
        private(&h.dir.join("catalog.json"),json!({"schema_version":1,"workspaces":[{"repository":"fixture/public","repository_id":99,"path":hint,"known_by":["coder"],"environment_paths":[]}]}).to_string().as_bytes());
        let o = h.invoke_role(true);
        assert_eq!(
            o.status.success(),
            moved,
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
        assert_eq!(
            std::fs::read_to_string(remote.join("code.py")).unwrap(),
            "value = 1\n"
        );
    }
}

#[test]
fn native_registry_model_and_instructions_reach_actual_cli_transport() {
    let mut h = Harness::new("model_override");
    h.req["adapter_policy"]["native_agent"]["agents"][0]["instructions"] =
        json!("Frozen registry operator instruction");
    h.req["adapter_policy"]["agent_config"] = json!({"revision":7,"coder":{"id":"coder","label":"Configured coder","role":"coder","instructions":"Frozen registry operator instruction","model":"registry-model","enabled":true}});
    assert_eq!(h.ok()["passed"], true);
}
