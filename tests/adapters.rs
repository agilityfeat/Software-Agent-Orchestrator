//! Synthetic real-adapter protocol tests: fake Docker/GitHub programs, real local Git. No external writes.
#![cfg(unix)]
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Stdio},
};
use uuid::Uuid;
struct Harness {
    _root: tempfile::TempDir,
    dir: PathBuf,
    request: Value,
}
impl Harness {
    fn new(mode: Value) -> Self {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().canonicalize().unwrap();
        std::fs::write(dir.join("mode.json"), mode.to_string()).unwrap();
        let state = dir.join("actions");
        std::fs::create_dir(&state).unwrap();
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).unwrap();
        let auth = dir.join("auth");
        std::fs::create_dir(&auth).unwrap();
        std::fs::write(
            auth.join("auth.json"),
            "{\"fixture\":\"synthetic-auth-only\"}",
        )
        .unwrap();
        std::fs::set_permissions(
            auth.join("auth.json"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let mut programs = Vec::new();
        for name in ["docker", "git", "gh"] {
            let p = dir.join(name);
            std::fs::write(&p, include_str!("fixtures/adapters.py")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
            programs.push(json!({"executable":p,"sha256":format!("{:x}",Sha256::digest(std::fs::read(&p).unwrap())),"credential_env_names":[]}));
        }
        let run = Uuid::new_v4().to_string();
        let workspace = dir.join(&run);
        let policy = json!({"repository":"fixture/private","state_root":state,"auth_home":auth,"docker":programs[0],"git":programs[1],"gh":programs[2],"docker_host":"unix:///synthetic.sock","worker_image":format!("sha256:{}","5".repeat(64)),"worker_network":"1".repeat(64),"proxy_container":"2".repeat(64),"proxy_image":format!("sha256:{}","3".repeat(64)),"proxy_ip":"192.0.2.2","proxy_port":3128,"proxy_policy_sha256":"4".repeat(64),"proxy_allowed_hosts":["chatgpt.com"],"model":null,"cpus":1,"memory_mb":256});
        let request = json!({"schema_version":1,"action":"prepare","action_id":Uuid::new_v4().to_string(),"run_id":run,"repository":"fixture/private","task_snapshot":{"repository":"fixture/private","workflow":"engineering","merge_policy":"auto","title":"Synthetic"},"deadline_ms":chrono::Utc::now().timestamp_millis()+60000,"stage_deadline_ms":chrono::Utc::now().timestamp_millis()+30000,"workspace":workspace,"base_sha":null,"head_sha":null,"base_ref":null,"qa_receipt":null,"merge_policy":"auto","observed_action_id":null,"approval_sha256":"6".repeat(64),"adapter_policy":policy});
        Self {
            _root: root,
            dir,
            request,
        }
    }
    fn invoke(&self, role: &str) -> std::process::Output {
        let exe = if role == "worker" {
            env!("CARGO_BIN_EXE_workboard-worker")
        } else {
            env!("CARGO_BIN_EXE_workboard-publisher")
        };
        let mut child = Command::new(exe)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("GH_TOKEN", "synthetic-token")
            .env("CODEX_HOME", self.dir.join("auth"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(self.request.to_string().as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
    fn ok(&self, role: &str) -> Value {
        let output = self.invoke(role);
        assert!(
            output.status.success(),
            "action={} error={} recent_synthetic_calls={:?}",
            self.request["action"],
            String::from_utf8_lossy(&output.stderr),
            self.audit().into_iter().rev().take(3).collect::<Vec<_>>()
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn stage(&mut self, action: &str) {
        self.request["action"] = json!(action);
        self.request["action_id"] = json!(Uuid::new_v4().to_string());
    }
    fn prepare(&mut self) {
        let r = self.ok("publisher");
        for key in ["base_sha", "head_sha", "base_ref"] {
            self.request[key] = r[key].clone();
        }
    }
    fn implement(&mut self) {
        self.stage("coder");
        let r = self.ok("worker");
        self.request["head_sha"] = r["head_sha"].clone();
    }
    fn qa(&mut self) -> Value {
        self.stage("qa");
        let qa = self.ok("worker");
        self.request["qa_receipt"] = qa.clone();
        qa
    }
    fn audit(&self) -> Vec<Value> {
        std::fs::read_to_string(self.dir.join("audit.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    }
}
#[test]
fn real_adapters_are_idempotent_and_publish_exact_reviewed_head() {
    let mut h = Harness::new(json!({}));
    h.prepare();
    h.implement();
    let qa = h.qa();
    assert_eq!(qa["passed"], true);
    assert!(qa.get("out_of_scope_bugs").is_none());
    assert_ne!(qa["session_id"], "coder-session-123456");
    h.stage("deliver");
    let result = h.ok("publisher");
    assert_eq!(result["merged"], true);
    assert_eq!(result["head_sha"], h.request["head_sha"]);
    let n = h.audit().len();
    assert_eq!(h.ok("publisher"), result);
    assert_eq!(h.audit().len(), n);
    assert!(h.audit().iter().all(|v| v["secret_in_args"] == false));
    let creates: Vec<_> = h
        .audit()
        .into_iter()
        .filter(|v| {
            v["tool"] == "docker" && v["args"].as_array().unwrap().iter().any(|a| a == "create")
        })
        .collect();
    assert_eq!(creates.len(), 2);
    for c in creates {
        let args = c["args"].as_array().unwrap();
        assert!(!args.iter().any(|v| v == "--init"));
        assert!(args.iter().any(|v| v == "--watchdog"));
        assert!(
            !args
                .iter()
                .any(|v| v.as_str().unwrap_or("").contains("docker.sock"))
        );
    }
}
#[test]
fn negative_qa_blocks_all_publication_and_manual_merge_never_calls_merge() {
    let mut h = Harness::new(json!({"qa_false":true}));
    h.prepare();
    h.implement();
    assert_eq!(h.qa()["passed"], false);
    h.stage("deliver");
    assert!(!h.invoke("publisher").status.success());
    assert!(
        !h.audit().iter().any(
            |v| v["tool"] == "git" && v["args"].as_array().unwrap().iter().any(|a| a == "push")
        )
    );
    let mut h = Harness::new(json!({}));
    h.request["merge_policy"] = json!("manual");
    h.request["task_snapshot"]["merge_policy"] = json!("manual");
    h.prepare();
    h.implement();
    h.qa();
    h.stage("deliver");
    assert_eq!(h.ok("publisher")["merged"], false);
    assert!(!h.audit().iter().any(|v| {
        v["tool"] == "gh"
            && v["args"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a.as_str().unwrap().ends_with("/merge"))
    }));
}
#[test]
fn uncertain_merge_observation_cannot_republish_or_release_lane() {
    let mut h = Harness::new(json!({"uncertain_merge":true}));
    h.prepare();
    h.implement();
    h.qa();
    h.stage("deliver");
    assert!(!h.invoke("publisher").status.success());
    let mutations = h
        .audit()
        .iter()
        .filter(|v| v["tool"] == "gh" && v["args"].as_array().unwrap().iter().any(|a| a == "PUT"))
        .count();
    h.stage("observe");
    assert!(!h.invoke("publisher").status.success());
    assert_eq!(
        h.audit()
            .iter()
            .filter(
                |v| v["tool"] == "gh" && v["args"].as_array().unwrap().iter().any(|a| a == "PUT")
            )
            .count(),
        mutations
    );
}
#[test]
fn state_symlink_is_rejected_without_chmod_and_deadline_changes_conflict() {
    let mut h = Harness::new(json!({}));
    let outside = h.dir.join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o755)).unwrap();
    let state = h
        .dir
        .join("actions")
        .join(h.request["run_id"].as_str().unwrap());
    std::os::unix::fs::symlink(&outside, &state).unwrap();
    assert!(!h.invoke("publisher").status.success());
    assert_eq!(
        std::fs::metadata(&outside).unwrap().permissions().mode() & 0o777,
        0o755
    );
    std::fs::remove_file(state).unwrap();
    h.prepare();
    h.request["deadline_ms"] = json!(h.request["deadline_ms"].as_i64().unwrap() + 1000);
    h.stage("observe");
    assert!(!h.invoke("publisher").status.success());
}
#[test]
fn publisher_askpass_is_repo_scoped_and_container_executor_refuses_host() {
    let exe = env!("CARGO_BIN_EXE_workboard-publisher");
    let output = Command::new(exe)
        .env_clear()
        .env("GH_TOKEN", "synthetic-token")
        .env("WORKBOARD_GIT_REPOSITORY", "fixture/private")
        .arg("Password for 'https://x-access-token@evil.example/fixture/private.git':")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let result = Command::new(env!("CARGO_BIN_EXE_workboard-contained"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("contained_environment_required"));
}

#[test]
fn existing_review_vetoes_and_unmergeable_heads_never_call_merge() {
    for mode in [
        json!({"review_veto":true}),
        json!({"unresolved_thread":true}),
        json!({"merge_conflict":true}),
    ] {
        let mut h = Harness::new(mode);
        h.prepare();
        h.implement();
        h.qa();
        h.stage("deliver");
        assert!(!h.invoke("publisher").status.success());
        assert!(
            !h.audit()
                .iter()
                .any(|v| v["tool"] == "gh"
                    && v["args"].as_array().unwrap().iter().any(|a| a == "PUT"))
        );
    }
}

#[test]
fn changed_provider_auth_is_quarantined_without_source_write_or_success() {
    let mut h = Harness::new(json!({"auth_rotate":true}));
    let source = std::fs::read(h.dir.join("auth/auth.json")).unwrap();
    h.prepare();
    h.stage("coder");
    let output = h.invoke("worker");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("adapter_auth_refresh_quarantined"));
    assert_eq!(std::fs::read(h.dir.join("auth/auth.json")).unwrap(), source);
    let action = h
        .dir
        .join("actions")
        .join(h.request["run_id"].as_str().unwrap())
        .join(h.request["action_id"].as_str().unwrap());
    let recovered: Value =
        serde_json::from_slice(&std::fs::read(action.join("auth-recovery.json")).unwrap()).unwrap();
    assert_eq!(recovered["fixture"], "rotated-synthetic-auth");
    assert_eq!(
        std::fs::metadata(action.join("auth-recovery.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(!action.join("receipt.json").exists());
    let state: Value =
        serde_json::from_slice(&std::fs::read(h.dir.join("state.json")).unwrap()).unwrap();
    assert!(state["containers"].as_array().unwrap().is_empty());
}

#[test]
fn rejected_export_cannot_chmod_or_read_external_host_file() {
    let mut h = Harness::new(json!({"validation_symlink":true}));
    let outside = h.dir.join("outside.json");
    std::fs::write(&outside, "{\"fixture\":true}").unwrap();
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o644)).unwrap();
    h.prepare();
    h.stage("coder");
    let output = h.invoke("worker");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("artifact_transfer"));
    assert_eq!(
        std::fs::metadata(&outside).unwrap().permissions().mode() & 0o777,
        0o644
    );
    assert_eq!(
        std::fs::read_to_string(outside).unwrap(),
        "{\"fixture\":true}"
    );
}

#[test]
fn contained_failure_is_retained_only_as_typed_facts_before_cleanup() {
    let mut h = Harness::new(json!({"contained_failure":true}));
    h.prepare();
    h.stage("coder");
    let output = h.invoke("worker");
    assert!(!output.status.success());
    let failure: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(failure["stage"], "codex");
    assert_eq!(failure["category"], "proxy_connection");
    assert_eq!(failure["exit_code"], 17);
    assert_eq!(failure["http_status"], 407);
    let action = h
        .dir
        .join("actions")
        .join(h.request["run_id"].as_str().unwrap())
        .join(h.request["action_id"].as_str().unwrap());
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(action.join("failure.json")).unwrap())
            .unwrap(),
        failure
    );
    assert!(!failure.to_string().contains("synthetic-private"));
    assert!(output.stdout.is_empty());
    assert!(!action.join("receipt.json").exists());
    assert!(!action.join("validation.json").exists());
    let state: Value =
        serde_json::from_slice(&std::fs::read(h.dir.join("state.json")).unwrap()).unwrap();
    assert!(state["containers"].as_array().unwrap().is_empty());
    let effects = h.audit().len();
    assert!(!h.invoke("worker").status.success());
    assert_eq!(h.audit().len(), effects);
}

#[test]
fn unsafe_export_evidence_is_rejected_before_host_persistence() {
    let mut h = Harness::new(json!({"unsafe_evidence":true}));
    h.prepare();
    h.stage("coder");
    let output = h.invoke("worker");
    assert!(!output.status.success());
    let action = h
        .dir
        .join("actions")
        .join(h.request["run_id"].as_str().unwrap())
        .join(h.request["action_id"].as_str().unwrap());
    assert!(!action.join("validation.json").exists());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("synthetic-private"));
}

fn select_node_profile(h: &mut Harness) {
    let root = std::path::Path::new(h.request["workspace"].as_str().unwrap());
    let hash = |file: &str| {
        format!(
            "{:x}",
            Sha256::digest(std::fs::read(root.join(file)).unwrap())
        )
    };
    h.request["adapter_policy"]["validation_profile"] = json!({"id":"node-npm-v1","manifest_sha256":hash("package.json"),"lock_sha256":hash("package-lock.json"),"check_sha256":hash("scripts/check.js"),"cache_sha256":"c".repeat(64)});
}
#[test]
fn node_profile_roundtrip_binds_real_patch_and_independent_qa_without_publication() {
    let mut h = Harness::new(json!({"node_profile":true}));
    h.prepare();
    select_node_profile(&mut h);
    h.implement();
    let state = std::path::Path::new(h.request["adapter_policy"]["state_root"].as_str().unwrap());
    let action = state
        .join(h.request["run_id"].as_str().unwrap())
        .join(h.request["action_id"].as_str().unwrap());
    let binding: Value =
        serde_json::from_slice(&std::fs::read(action.join("validation-binding.json")).unwrap())
            .unwrap();
    assert_eq!(binding["validated_head_sha"], h.request["head_sha"]);
    h.stage("qa");
    let receipt = h.ok("worker");
    assert_eq!(receipt["passed"], true);
    assert_eq!(receipt["reviewed_sha"], h.request["head_sha"]);
    assert!(
        !h.audit().iter().any(
            |v| v["tool"] == "git" && v["args"].as_array().unwrap().iter().any(|a| a == "push")
        )
    );
}
#[test]
fn node_profile_refuses_unknown_drift_preflight_legacy_zero_tests_and_forged_provenance() {
    for mode in [
        "unknown_profile",
        "lock_drift",
        "preflight_evidence",
        "legacy_node_evidence",
        "wrong_profile_cache",
        "wrong_profile_head",
        "wrong_profile_tree",
        "zero_node_tests",
    ] {
        let mut h = Harness::new(json!({"node_profile":true,mode:true}));
        h.prepare();
        select_node_profile(&mut h);
        h.stage("coder");
        if mode == "unknown_profile" {
            h.request["adapter_policy"]["validation_profile"]["id"] = json!("unknown");
        }
        if mode == "lock_drift" {
            std::fs::write(
                std::path::Path::new(h.request["workspace"].as_str().unwrap())
                    .join("package-lock.json"),
                "drift",
            )
            .unwrap();
        }
        let before = h.audit().len();
        let result = h.invoke("worker");
        assert!(!result.status.success(), "{mode}");
        if ["unknown_profile", "lock_drift"].contains(&mode) {
            assert_eq!(
                before,
                h.audit().len(),
                "{mode} must fail before Docker/program dispatch"
            );
        }
    }
}

#[test]
fn node_cleanup_does_not_require_a_valid_project_manifest() {
    let mut h = Harness::new(json!({"node_profile":true}));
    h.prepare();
    select_node_profile(&mut h);
    std::fs::write(
        std::path::Path::new(h.request["workspace"].as_str().unwrap()).join("package-lock.json"),
        "interrupted dirty manifest",
    )
    .unwrap();
    h.stage("cancel");
    assert_eq!(h.ok("worker")["cleanup_complete"], true);
}

#[test]
fn node_cancel_before_prepare_does_not_require_workspace_or_manifests() {
    let mut h = Harness::new(json!({}));
    h.request["adapter_policy"]["validation_profile"] = json!({"id":"node-npm-v1","manifest_sha256":"a".repeat(64),"lock_sha256":"b".repeat(64),"check_sha256":"c".repeat(64),"cache_sha256":"d".repeat(64)});
    assert!(!std::path::Path::new(h.request["workspace"].as_str().unwrap()).exists());
    h.stage("cancel");
    assert_eq!(h.ok("worker")["cleanup_complete"], true);
    assert!(!h.audit().iter().any(|v| v["tool"] == "git"));
}

#[test]
fn completed_negative_qa_keeps_concrete_findings_and_truthful_failed_validation() {
    let mut h = Harness::new(json!({"qa_false":true,"qa_findings":true,"failed_validation":true}));
    h.prepare();
    h.implement();
    let qa = h.qa();
    assert_eq!(qa["passed"], false);
    assert_eq!(qa["qa_completed"], true);
    assert_eq!(qa["findings"][0]["path"], "f.txt");
    assert_eq!(qa["cleanup_complete"], true);
    assert!(h.audit().iter().all(|v| v["secret_in_args"] == false));
    let mut incomplete =
        Harness::new(json!({"qa_false":true,"qa_findings":true,"incomplete_qa":true}));
    incomplete.prepare();
    incomplete.implement();
    incomplete.stage("qa");
    assert!(!incomplete.invoke("worker").status.success());
}

#[test]
fn signalled_python_validation_is_not_an_actionable_completed_qa_rejection() {
    let mut h = Harness::new(
        json!({"qa_false":true,"qa_findings":true,"failed_validation":true,"signalled_validation":true}),
    );
    h.prepare();
    h.implement();
    h.stage("qa");
    assert!(!h.invoke("worker").status.success());
}

#[test]
fn completed_positive_qa_accepts_explicit_empty_findings() {
    let mut h = Harness::new(json!({"qa_empty_findings":true}));
    h.prepare();
    h.implement();
    let qa = h.qa();
    assert_eq!(qa["passed"], true);
    assert_eq!(qa["cleanup_complete"], true);
    assert!(qa.get("findings").is_none());
}

#[test]
fn qa_rejection_diagnostics_are_precise_without_retaining_unvalidated_data_or_retrying() {
    for (mode, code, retained) in [
        (
            json!({"qa_result_schema_invalid":true}),
            "adapter_worker_result_schema_invalid",
            false,
        ),
        (
            json!({"qa_unsafe_evidence":true}),
            "adapter_validation_evidence_invalid",
            false,
        ),
        (
            json!({"qa_evidence_session_mismatch":true}),
            "adapter_validation_session_mismatch",
            true,
        ),
        (
            json!({"qa_turn_incomplete":true}),
            "adapter_validation_turn_incomplete",
            true,
        ),
        (
            json!({"qa_success_missing":true}),
            "adapter_validation_success_missing",
            true,
        ),
        (json!({"incomplete_qa":true}), "adapter_qa_incomplete", true),
        (
            json!({"qa_findings":true}),
            "adapter_qa_feedback_invalid",
            true,
        ),
        (
            json!({"qa_false":true,"qa_findings":true,"failed_validation":true,"signalled_validation":true}),
            "adapter_qa_validation_incomplete",
            true,
        ),
    ] {
        let mut h = Harness::new(mode);
        h.prepare();
        h.implement();
        h.stage("qa");
        let output = h.invoke("worker");
        assert!(!output.status.success(), "{code}");
        let failure: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(failure["code"], code);
        assert_eq!(failure["category"], "invalid_result");
        assert!(output.stdout.is_empty());
        let action = h
            .dir
            .join("actions")
            .join(h.request["run_id"].as_str().unwrap())
            .join(h.request["action_id"].as_str().unwrap());
        assert!(!action.join("receipt.json").exists());
        assert!(!action.join("validation.json").exists(), "{code}");
        assert_eq!(
            action.join("validation-status.json").exists(),
            retained,
            "{code}"
        );
        if retained {
            let status: Value = serde_json::from_slice(
                &std::fs::read(action.join("validation-status.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(status.as_object().unwrap().len(), 11);
            assert!(status["result_completed"].is_boolean());
            assert!(status["session_matches"].is_boolean());
            assert!(status["successful_tests"].is_u64());
            if code == "adapter_qa_validation_incomplete" {
                assert_eq!(status["test_exit_code"], -1);
            }
        }
        for entry in std::fs::read_dir(&action).unwrap() {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains("synthetic-private"));
        }
        let audit_before = h.audit().len();
        assert!(!h.invoke("worker").status.success());
        assert_eq!(
            h.audit().len(),
            audit_before,
            "an uncertain action must not replay"
        );
        let state: Value =
            serde_json::from_slice(&std::fs::read(h.dir.join("state.json")).unwrap()).unwrap();
        assert_eq!(state["containers"], json!([]));
    }
}

#[test]
fn scoped_qa_reports_pass_and_mixed_negative_preserve_only_valid_unrelated_data() {
    for negative in [false, true] {
        let mut h = Harness::new(
            json!({"qa_out_of_scope":true,"qa_empty_findings":!negative,"qa_findings":negative,"qa_false":negative,"failed_validation":negative}),
        );
        h.prepare();
        h.implement();
        let qa = h.qa();
        assert_eq!(qa["passed"], !negative);
        assert_eq!(qa["qa_completed"], true);
        assert_eq!(qa["out_of_scope_bugs"][0]["criticality"], "high");
        assert_eq!(
            qa["out_of_scope_bugs"][0]["base_sha"],
            h.request["base_sha"]
        );
        assert_eq!(qa["cleanup_complete"], true);
    }
}
#[test]
fn scoped_qa_cannot_report_on_incomplete_infra_or_foreign_authority() {
    for mode in [
        json!({"qa_out_of_scope":true,"incomplete_qa":true}),
        json!({"qa_out_of_scope":true,"out_scope_wrong_base":true}),
        json!({"qa_out_of_scope":true,"out_scope_injection":"repository"}),
        json!({"qa_out_of_scope":true,"out_scope_injection":"status"}),
        json!({"qa_out_of_scope":true,"qa_false":true}),
        json!({"qa_out_of_scope":true,"qa_false":true,"qa_findings":true,"failed_validation":true,"signalled_validation":true}),
    ] {
        let mut h = Harness::new(mode);
        h.prepare();
        h.implement();
        h.stage("qa");
        let result = h.invoke("worker");
        assert!(!result.status.success());
        let action = h
            .dir
            .join("actions")
            .join(h.request["run_id"].as_str().unwrap())
            .join(h.request["action_id"].as_str().unwrap());
        assert!(!action.join("receipt.json").exists());
    }
}

#[test]
fn actual_planner_adapter_has_no_source_mounts_or_git_and_requires_valid_completed_result() {
    for invalid in [false, true] {
        let mut h = Harness::new(json!({"bad_plan":invalid}));
        h.stage("plan");
        h.request["task_snapshot"]["goal"] = json!("A bounded planning goal");
        let result = h.invoke("worker");
        assert_eq!(result.status.success(), !invalid);
        if !invalid {
            let receipt: Value = serde_json::from_slice(&result.stdout).unwrap();
            assert_eq!(receipt["role"], "plan");
            assert_eq!(receipt["cleanup_complete"], true);
            assert_eq!(receipt["plan"]["tasks"][0]["priority"], 3);
            assert!(receipt["head_sha"].is_null());
        }
        let audit = h.audit();
        assert!(audit.iter().all(|a| a["tool"] == "docker"));
        let create = audit
            .iter()
            .find(|a| a["args"].as_array().unwrap().iter().any(|v| v == "create"))
            .unwrap();
        assert!(
            !create["args"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "--mount")
        );
        assert!(
            create["args"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "/workspace:ro,noexec,nosuid,nodev,size=1048576,mode=0755")
        );
        assert!(audit.iter().all(|a| {
            !a["env_names"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "GH_TOKEN" || v == "GITHUB_TOKEN")
        }));
    }
}

#[test]
fn actual_coder_skip_receipt_is_required_for_delivery_and_manual_merge_is_preserved() {
    for merge in ["auto", "manual"] {
        let mut h = Harness::new(json!({}));
        h.request["task_snapshot"]["qa_policy"] = json!("skip");
        h.request["task_snapshot"]["merge_policy"] = json!(merge);
        h.request["merge_policy"] = json!(merge);
        h.prepare();
        h.stage("coder");
        let coder = h.ok("worker");
        assert_eq!(coder["passed"], true);
        assert_eq!(coder["cleanup_complete"], true);
        h.request["head_sha"] = coder["head_sha"].clone();
        h.request["coder_receipt"] = coder.clone();
        h.stage("deliver");
        let delivered = h.ok("publisher");
        assert_eq!(delivered["merged"], merge == "auto");
        let state: Value =
            serde_json::from_slice(&std::fs::read(h.dir.join("state.json")).unwrap()).unwrap();
        assert!(
            state["pr_body"]
                .as_str()
                .unwrap()
                .contains("QA explicitly skipped")
        );
        assert!(!state["pr_body"].as_str().unwrap().contains("QA passed"));
        assert!(!h.audit().iter().any(|v| {
            v["args"]
                .as_array()
                .is_some_and(|a| a.iter().any(|arg| arg == "qa"))
        }));
    }
    for corrupt in ["missing", "head", "passed", "cleanup"] {
        let mut h = Harness::new(json!({}));
        h.request["task_snapshot"]["qa_policy"] = json!("skip");
        h.prepare();
        h.stage("coder");
        let mut coder = h.ok("worker");
        h.request["head_sha"] = coder["head_sha"].clone();
        match corrupt {
            "head" => coder["head_sha"] = json!("0".repeat(40)),
            "passed" => coder["passed"] = json!(false),
            "cleanup" => coder["cleanup_complete"] = json!(false),
            _ => {}
        }
        if corrupt != "missing" {
            h.request["coder_receipt"] = coder;
        }
        h.stage("deliver");
        assert!(!h.invoke("publisher").status.success());
        assert!(!h.audit().iter().any(|v| {
            v["tool"] == "gh"
                && v["args"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|arg| arg == "POST" || arg == "PUT"))
        }));
    }
}

#[test]
fn no_install_unit_profile_roundtrip_and_adverse_evidence_use_real_host_adapters() {
    for mode in [
        "valid",
        "zero_node_tests",
        "wrong_profile_head",
        "wrong_profile_tree",
        "preflight_evidence",
    ] {
        let mut h = Harness::new(json!({"unit_profile":true,mode:true}));
        h.prepare();
        let root = std::path::Path::new(h.request["workspace"].as_str().unwrap());
        h.request["adapter_policy"]["validation_profile"] = json!({"id":"node-unit-syntax-v1","manifest_sha256":format!("{:x}",Sha256::digest(std::fs::read(root.join("package.json")).unwrap()))});
        h.stage("coder");
        if mode != "valid" {
            assert!(!h.invoke("worker").status.success(), "{mode}");
            continue;
        }
        let coder = h.ok("worker");
        h.request["head_sha"] = coder["head_sha"].clone();
        let qa = h.qa();
        assert_eq!(qa["passed"], true);
        assert_eq!(qa["reviewed_sha"], h.request["head_sha"]);
        h.stage("deliver");
        assert_eq!(h.ok("publisher")["merged"], true);
        assert!(
            h.audit().iter().all(|e| !e["args"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a == "npm" || a == "ci" || a == "install" || a == "build"))
        );
    }
}

#[test]
fn contained_registry_context_and_model_reach_planning_and_worker_input() {
    for action in ["plan", "coder"] {
        let mut h = Harness::new(json!({}));
        if action == "coder" {
            h.prepare();
        }
        h.stage(action);
        let role = if action == "plan" { "main" } else { "coder" };
        h.request["adapter_policy"]["agent_config"] = json!({"revision":2,role:{"id":role,"label":"Configured agent","role":role,"instructions":"Frozen content only","model":"registry-model","enabled":true}});
        h.ok("worker");
        let state: Value =
            serde_json::from_slice(&std::fs::read(h.dir.join("state.json")).unwrap()).unwrap();
        assert_eq!(state["contained_input"]["model"], "registry-model");
        assert_eq!(
            state["contained_input"]["agent_context"]["instructions"],
            "Frozen content only"
        );
    }
}
