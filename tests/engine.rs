//! SYNTHETIC ORCHESTRATION / NON_ACCEPTANCE. Real subprocesses and local Git only.
//! The fixture launcher is not a production containment or provider acceptance artifact.
#![cfg(unix)]
use axum::{body::Body, http::Request, response::Response};
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sha2::Sha256;
use std::{os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};
use tower::ServiceExt;
use workboard_rs::{
    App,
    execution_config::{Approval, digest_file, now_ms},
};
const KEY: &str = "synthetic-test-key-0123456789abcdef";
struct Harness {
    root: tempfile::TempDir,
    dir: PathBuf,
    app: Option<App>,
    engine: Option<tokio::task::JoinHandle<()>>,
    cookie: String,
    csrf: String,
    digest: String,
}
impl Harness {
    async fn new(mode: Value, stage_seconds: u64, start: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().canonicalize().unwrap();
        let work = dir.join("work");
        std::fs::create_dir(&work).unwrap();
        std::fs::set_permissions(&work, std::fs::Permissions::from_mode(0o700)).unwrap();
        let driver = dir.join("driver.py");
        std::fs::write(&driver, include_str!("fixtures/driver.py")).unwrap();
        std::fs::set_permissions(&driver, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(dir.join("mode.json"), mode.to_string()).unwrap();
        let program = json!({"executable":driver,"sha256":digest_file(&driver).unwrap(),"credential_env_names":[]});
        let git_path = if mode["hold_preflight"] == true {
            let path = dir.join("git_gate.py");
            std::fs::write(&path, include_str!("fixtures/git_gate.py")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            std::fs::write(dir.join("hold-preflight"), b"synthetic barrier").unwrap();
            path
        } else {
            PathBuf::from("/usr/bin/git")
        };
        let git = json!({"executable":git_path,"sha256":digest_file(&git_path).unwrap(),"credential_env_names":[]});
        let mut payload = json!({"schema_version":1,"containment_contract":"reviewed-launcher-v1","expires_at_ms":now_ms()+mode["approval_lifetime_ms"].as_i64().unwrap_or(3600000),"work_root":work,"worker":program,"publisher":program,"git":git,"max_run_seconds":mode["run_seconds"].as_u64().unwrap_or(60),"max_stage_seconds":stage_seconds,"max_output_bytes":8192});
        if let Some(max) = mode.get("max_qa_repairs") {
            payload["max_qa_repairs"] = max.clone();
        }
        let mut mac = Hmac::<Sha256>::new_from_slice(KEY.as_bytes()).unwrap();
        mac.update(payload.to_string().as_bytes());
        let signature = format!("{:x}", mac.finalize().into_bytes());
        let envelope = json!({"payload":payload,"signature":signature});
        let config = dir.join("approval.json");
        std::fs::write(&config, envelope.to_string()).unwrap();
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
        let digest = digest_file(&config).unwrap();
        let approval = Approval::load(&config, KEY.as_bytes(), &digest).unwrap();
        let app = App::open_approved(&dir.join("board.sqlite"), KEY, 4311, Some(approval)).unwrap();
        let engine = if start {
            Some(app.start_execution())
        } else {
            None
        };
        let mut h = Self {
            root,
            dir,
            app: Some(app),
            engine,
            cookie: String::new(),
            csrf: String::new(),
            digest,
        };
        h.login().await;
        h
    }
    fn app(&self) -> &App {
        self.app.as_ref().unwrap()
    }
    async fn login(&mut self) {
        let r = self
            .request("POST", "/session", Some(json!({"key":KEY})), None, false)
            .await;
        assert_eq!(r.status(), 200);
        self.cookie = r.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        self.csrf = body(r).await["csrf_token"].as_str().unwrap().to_string();
    }
    async fn request(
        &self,
        method: &str,
        path: &str,
        value: Option<Value>,
        key: Option<&str>,
        signed: bool,
    ) -> Response {
        let mut r = Request::builder()
            .method(method)
            .uri(format!("/api/workboard/v1{path}"))
            .header("host", "127.0.0.1:4311");
        if method != "GET" {
            r = r
                .header("origin", "http://127.0.0.1:4311")
                .header("content-type", "application/json");
        }
        if signed {
            r = r
                .header("cookie", &self.cookie)
                .header("x-csrf-token", &self.csrf);
        }
        if let Some(key) = key {
            r = r.header("idempotency-key", key);
        }
        self.app()
            .router()
            .oneshot(
                r.body(Body::from(value.map(|v| v.to_string()).unwrap_or_default()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }
    async fn command(&self, path: &str, value: Value, key: &str, status: u16) -> Value {
        let r = self
            .request("POST", path, Some(value), Some(key), true)
            .await;
        let actual = r.status().as_u16();
        let v = body(r).await;
        assert_eq!(actual, status, "{path}: {v}");
        v
    }
    async fn snapshot(&self) -> Value {
        body(self.request("GET", "/snapshot", None, None, true).await).await
    }
    async fn create(&self, merge: &str, key: &str) -> Value {
        self.command("/tasks",json!({"title":"Synthetic only","objective":"Validate orchestration without providers or GitHub","repository":"fixture/repository","workflow":"engineering","status":"ready","merge_policy":merge}),key,201).await["task"].clone()
    }
    async fn run(&self, task: &Value, key: &str) -> Value {
        self.command(
            &format!("/tasks/{}/run", task["id"].as_str().unwrap()),
            json!({"expected_revision":task["revision"]}),
            key,
            202,
        )
        .await["run"]
            .clone()
    }
    async fn unpause(&self) {
        self.command(
            "/execution/pause",
            json!({"paused":false}),
            "unpause-fixture",
            200,
        )
        .await;
    }
    async fn wait(&self, statuses: &[&str]) -> Value {
        tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                let snap = self.snapshot().await;
                let runs = snap["runs"].as_array().unwrap();
                if let Some(run) = runs.last() {
                    if statuses.contains(&run["status"].as_str().unwrap()) {
                        return run.clone();
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "run did not reach expected {statuses:?}; audit={:?}",
                self.audit()
            )
        })
    }
    fn audit(&self) -> Vec<Value> {
        std::fs::read_to_string(self.dir.join("audit.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|v| serde_json::from_str(v).unwrap())
            .collect()
    }
    async fn wait_action(&self, action: &str) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while !self.audit().iter().any(|v| v["action"] == action) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }
    async fn restart(&mut self) {
        self.stop().await;
        let approval = Approval::load(
            &self.dir.join("approval.json"),
            KEY.as_bytes(),
            &self.digest,
        )
        .unwrap();
        let app =
            App::open_approved(&self.dir.join("board.sqlite"), KEY, 4311, Some(approval)).unwrap();
        self.engine = Some(app.start_execution());
        self.app = Some(app);
        self.login().await;
    }
    async fn stop(&mut self) {
        if let Some(app) = self.app.take() {
            app.shutdown();
            if let Some(engine) = self.engine.take() {
                tokio::time::timeout(Duration::from_secs(5), engine)
                    .await
                    .unwrap()
                    .unwrap();
            }
            drop(app);
        }
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(app) = self.app.as_ref() {
            app.shutdown();
        }
        let _ = &self.root;
    }
}
async fn body(r: Response) -> Value {
    serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[tokio::test]
async fn real_process_pipeline_is_exactly_once_and_sha_bound() {
    let mut h = Harness::new(json!({}), 10, true).await;
    assert_eq!(
        App::open(&h.dir.join("board.sqlite"), KEY, 4312)
            .err()
            .unwrap()
            .code(),
        "database_already_owned"
    );
    let task = h.create("auto", "create-auto").await;
    assert_eq!(h.snapshot().await["capabilities"]["run"], false);
    assert!(h.audit().is_empty());
    h.unpause().await;
    let run = h.run(&task, "run-auto").await;
    let replay = h.run(&task, "run-auto").await;
    assert_eq!(run, replay);
    let second = h.create("auto", "create-second").await;
    h.command(
        &format!("/tasks/{}/run", second["id"].as_str().unwrap()),
        json!({"expected_revision":1}),
        "run-second",
        409,
    )
    .await;
    let finished = h.wait(&["succeeded", "unknown", "failed"]).await;
    assert_eq!(finished["status"], "succeeded", "{finished}");
    assert_eq!(finished["receipt_count"], 4);
    assert_eq!(finished["approval_sha256"], h.digest);
    assert_eq!(finished["evidence"]["qa_status"], "passed");
    assert_eq!(
        h.audit()
            .iter()
            .map(|v| v["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["prepare", "coder", "qa", "deliver"]
    );
    let snap = h.snapshot().await;
    assert!(snap["execution"]["lane_run_id"].is_null());
    assert!(
        snap["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["id"] == task["id"] && v["status"] == "done")
    );
    let completed = snap["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == task["id"])
        .unwrap();
    let archived = h
        .command(
            &format!("/tasks/{}/archive", task["id"].as_str().unwrap()),
            json!({"expected_revision":completed["revision"]}),
            "archive-engineering-done",
            200,
        )
        .await;
    assert_eq!(archived["task"]["status"], "done");
    let restored = h
        .command(
            &format!("/tasks/{}/restore", task["id"].as_str().unwrap()),
            json!({"expected_revision":archived["task"]["revision"]}),
            "restore-engineering-done",
            200,
        )
        .await;
    assert_eq!(restored["task"]["completed_at"], completed["completed_at"]);
    h.command(
        &format!("/tasks/{}/reopen", task["id"].as_str().unwrap()),
        json!({"expected_revision":restored["task"]["revision"]}),
        "reopen-engineering-done",
        200,
    )
    .await;
    h.stop().await;
}

#[tokio::test]
async fn adverse_worker_receipts_and_git_config_stop_before_publication() {
    for mode in [
        json!({"qa_failed":true}),
        json!({"qa_verdict_false":true}),
        json!({"wrong_qa_sha":true}),
        json!({"same_qa_session":true}),
        json!({"unknown_fields":"qa"}),
        json!({"malicious_fsmonitor":true}),
        json!({"external_gitdir":true}),
    ] {
        let mut h = Harness::new(mode.clone(), 10, true).await;
        h.unpause().await;
        let task = h.create("auto", "create-adverse").await;
        h.run(&task, "run-adverse").await;
        let stopped = h.wait(&["unknown", "failed"]).await;
        assert_ne!(stopped["status"], "succeeded");
        if mode["qa_failed"] == true || mode["qa_verdict_false"] == true {
            assert_eq!(stopped["status"], "failed");
            assert_eq!(stopped["reason"], "qa_failed");
            assert_eq!(stopped["evidence"]["qa_status"], "failed");
        }
        assert!(
            !h.audit().iter().any(|v| v["action"] == "deliver"),
            "mode={mode}"
        );
        assert!(!h.dir.join("fsmonitor-ran.txt").exists());
        h.stop().await;
    }
}

#[tokio::test]
async fn manual_merge_requires_explicit_observation() {
    let mut h = Harness::new(json!({"observe_pending":true}), 10, true).await;
    h.unpause().await;
    let task = h.create("manual", "create-manual").await;
    let run = h.run(&task, "run-manual").await;
    h.wait(&["awaiting_merge"]).await;
    assert!(!h.snapshot().await["execution"]["lane_run_id"].is_null());
    assert_eq!(
        h.audit()
            .iter()
            .filter(|v| v["action"] == "deliver")
            .count(),
        1
    );
    h.restart().await;
    assert_eq!(h.snapshot().await["runs"][0]["status"], "unknown");
    h.command(
        &format!("/runs/{}/reconcile", run["id"].as_str().unwrap()),
        json!({}),
        "observe-pending-after-restart",
        202,
    )
    .await;
    h.wait(&["awaiting_merge"]).await;
    assert!(!h.snapshot().await["execution"]["lane_run_id"].is_null());
    // An operator's merge must still identify the exact reviewed implementation.
    std::fs::write(
        h.dir.join("mode.json"),
        json!({"observed_merged":true,"observed_wrong_head":true}).to_string(),
    )
    .unwrap();
    h.command(
        &format!("/runs/{}/reconcile", run["id"].as_str().unwrap()),
        json!({}),
        "observe-wrong-head",
        202,
    )
    .await;
    let uncertain = h.wait(&["unknown"]).await;
    assert_eq!(uncertain["reason"], "delivery_receipt_invalid");
    assert_eq!(h.snapshot().await["execution"]["lane_run_id"], run["id"]);
    assert_ne!(h.snapshot().await["tasks"][0]["status"], "done");
    std::fs::write(
        h.dir.join("mode.json"),
        json!({"observed_merged":true}).to_string(),
    )
    .unwrap();
    h.command(
        &format!("/runs/{}/reconcile", run["id"].as_str().unwrap()),
        json!({}),
        "observe-merge",
        202,
    )
    .await;
    assert_eq!(h.wait(&["succeeded"]).await["status"], "succeeded");
    assert_eq!(
        h.audit()
            .iter()
            .filter(|v| v["action"] == "deliver")
            .count(),
        1
    );
    h.stop().await;
}

#[tokio::test]
async fn cancellation_cleans_dirty_worker_even_while_paused() {
    let mut h = Harness::new(
        json!({"dirty_before_sleep":true,"sleep_coder":30}),
        40,
        true,
    )
    .await;
    h.unpause().await;
    let task = h.create("auto", "create-cancel").await;
    let run = h.run(&task, "run-cancel").await;
    h.wait_action("coder").await;
    h.command(
        "/execution/pause",
        json!({"paused":true}),
        "pause-before-stop",
        200,
    )
    .await;
    h.command(
        &format!("/runs/{}/cancel", run["id"].as_str().unwrap()),
        json!({}),
        "cancel-worker",
        202,
    )
    .await;
    let stopped = h.wait(&["cancelled", "unknown"]).await;
    assert_eq!(stopped["status"], "cancelled", "{stopped}");
    assert_eq!(
        h.audit()
            .iter()
            .map(|v| v["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["prepare", "coder", "cancel", "observe"]
    );
    assert!(h.snapshot().await["execution"]["lane_run_id"].is_null());
    h.stop().await;
}

#[tokio::test]
async fn restart_never_replays_uncertain_action_and_reconcile_only_observes() {
    let mut h = Harness::new(
        json!({"dirty_before_sleep":true,"sleep_coder":30}),
        40,
        true,
    )
    .await;
    h.unpause().await;
    let task = h.create("auto", "create-restart").await;
    let run = h.run(&task, "run-restart").await;
    h.wait_action("coder").await;
    h.stop().await;
    let approval = Approval::load(&h.dir.join("approval.json"), KEY.as_bytes(), &h.digest).unwrap();
    let app = App::open_approved(&h.dir.join("board.sqlite"), KEY, 4311, Some(approval)).unwrap();
    h.engine = Some(app.start_execution());
    h.app = Some(app);
    h.login().await;
    assert_eq!(h.snapshot().await["runs"][0]["status"], "unknown");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        h.audit().iter().filter(|v| v["action"] == "coder").count(),
        1
    );
    let second = h.create("auto", "create-busy-after-restart").await;
    let rejected = h
        .command(
            &format!("/tasks/{}/run", second["id"].as_str().unwrap()),
            json!({"expected_revision":second["revision"]}),
            "busy-after-restart",
            409,
        )
        .await;
    assert_eq!(rejected["error"], "active_run_exists");
    assert_eq!(h.snapshot().await["execution"]["lane_run_id"], run["id"]);
    assert_eq!(h.run(&task, "run-restart").await["id"], run["id"]);
    h.command(
        "/execution/pause",
        json!({"paused":true}),
        "pause-before-recovery-observation",
        200,
    )
    .await;
    h.command(
        &format!("/runs/{}/reconcile", run["id"].as_str().unwrap()),
        json!({}),
        "reconcile-restart",
        202,
    )
    .await;
    assert_eq!(h.wait(&["failed"]).await["status"], "failed");
    assert_eq!(
        h.audit().iter().filter(|v| v["action"] == "coder").count(),
        1
    );
    assert_eq!(h.audit().last().unwrap()["action"], "observe");
    h.stop().await;
}

#[tokio::test]
async fn pin_drift_and_timeout_remain_unknown_without_retry() {
    let mut h = Harness::new(json!({}), 10, false).await;
    h.unpause().await;
    let task = h.create("auto", "create-pin").await;
    h.run(&task, "run-pin-drift").await;
    let driver = h.dir.join("driver.py");
    let mut bytes = std::fs::read(&driver).unwrap();
    bytes.extend_from_slice(b"\n# changed pin\n");
    std::fs::write(driver, bytes).unwrap();
    h.engine = Some(h.app().start_execution());
    assert_eq!(
        h.wait(&["unknown"]).await["reason"],
        "execution_pin_invalid"
    );
    assert!(h.audit().is_empty());
    h.stop().await;
    let mut h = Harness::new(json!({"sleep_coder":30}), 1, true).await;
    h.unpause().await;
    let task = h.create("auto", "create-timeout").await;
    h.run(&task, "run-timeout").await;
    let stopped = h.wait(&["unknown"]).await;
    assert_eq!(stopped["reason"], "deadline_exceeded");
    assert!(!h.snapshot().await["execution"]["lane_run_id"].is_null());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        h.audit().iter().filter(|v| v["action"] == "coder").count(),
        1
    );
    h.stop().await;
}

#[tokio::test]
async fn expired_signed_approval_keeps_cleanup_available_and_rejects_scope_expansion() {
    let mut h = Harness::new(json!({"approval_lifetime_ms":300}), 10, false).await;
    tokio::time::sleep(Duration::from_millis(350)).await;
    let snapshot = h.snapshot().await;
    assert_eq!(snapshot["capabilities"]["run"], false);
    assert_eq!(snapshot["capabilities"]["stop"], true);
    assert_eq!(snapshot["capabilities"]["cancel"], true);
    let path = h.dir.join("approval.json");
    let mut envelope: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    envelope["payload"]["worker"]["credential_env_names"] = json!(["GH_TOKEN"]);
    let mut mac = Hmac::<Sha256>::new_from_slice(KEY.as_bytes()).unwrap();
    mac.update(envelope["payload"].to_string().as_bytes());
    envelope["signature"] = json!(format!("{:x}", mac.finalize().into_bytes()));
    std::fs::write(&path, envelope.to_string()).unwrap();
    let digest = digest_file(&path).unwrap();
    assert_eq!(
        Approval::load(&path, KEY.as_bytes(), &digest)
            .err()
            .unwrap()
            .code(),
        "execution_credential_scope_invalid"
    );
    assert!(h.audit().is_empty());
    h.stop().await;
}

#[tokio::test]
async fn failed_cleanup_attempts_stop_unknown_without_automatic_retry() {
    for failed_action in ["cancel", "observe"] {
        let mut h = Harness::new(
            json!({"dirty_before_sleep":true,"sleep_coder":30,"bad_receipt":failed_action}),
            40,
            true,
        )
        .await;
        h.unpause().await;
        let task = h.create("auto", "create-cleanup-failure").await;
        let run = h.run(&task, "run-cleanup-failure").await;
        h.wait_action("coder").await;
        h.command(
            &format!("/runs/{}/cancel", run["id"].as_str().unwrap()),
            json!({}),
            "cancel-cleanup-failure",
            202,
        )
        .await;
        h.wait(&["unknown"]).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            h.audit()
                .iter()
                .filter(|v| v["action"] == failed_action)
                .count(),
            1
        );
        assert!(!h.snapshot().await["execution"]["lane_run_id"].is_null());
        h.stop().await;
    }
}

#[tokio::test]
async fn typed_driver_failures_are_safe_durable_and_never_relaunch() {
    for mode in [
        json!({"typed_failure":true}),
        json!({"typed_failure":true,"unsafe_failure":true}),
        json!({"typed_failure":true,"oversize_failure":true}),
        json!({"stdout_failure":true}),
    ] {
        let unsafe_fields = mode["unsafe_failure"] == true
            || mode["oversize_failure"] == true
            || mode["stdout_failure"] == true;
        let mut mode = mode;
        mode["driver_failure"] = json!(true);
        let mut h = Harness::new(mode, 10, true).await;
        h.unpause().await;
        let task = h.create("auto", "create-failure").await;
        let admitted = h.run(&task, "run-failure").await;
        let stopped = h.wait(&["unknown"]).await;
        assert_eq!(stopped["reason"], "driver_transport_failed");
        assert_eq!(stopped["diagnostic"]["exit_code"], 7);
        assert_eq!(
            stopped["diagnostic"]["stage"],
            if unsafe_fields { "driver" } else { "codex" }
        );
        if !unsafe_fields {
            assert_eq!(stopped["diagnostic"]["category"], "authentication");
            assert_eq!(stopped["diagnostic"]["http_status"], 401);
            assert_eq!(stopped["diagnostic"]["session_started"], true);
        }
        assert!(!h.snapshot().await.to_string().contains("synthetic-private"));
        assert_eq!(
            h.snapshot().await["execution"]["lane_run_id"],
            admitted["id"]
        );
        h.run(&task, "run-failure").await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            h.audit()
                .iter()
                .map(|v| v["action"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["prepare", "coder"]
        );
        h.restart().await;
        assert_eq!(
            h.snapshot().await["runs"][0]["diagnostic"],
            stopped["diagnostic"]
        );
        h.stop().await;
    }
}

#[tokio::test]
async fn pause_during_preflight_defers_spawn_until_explicit_resume() {
    let mut h = Harness::new(json!({"hold_preflight":true}), 15, true).await;
    h.unpause().await;
    let task = h.create("auto", "create-preflight-pause").await;
    let run = h.run(&task, "run-preflight-pause").await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !h.dir.join("preflight-waiting").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    h.command(
        "/execution/pause",
        json!({"paused":true}),
        "pause-during-preflight",
        200,
    )
    .await;
    let paused_watermark = h.snapshot().await["watermark"].as_u64().unwrap();
    std::fs::remove_file(h.dir.join("hold-preflight")).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while h.snapshot().await["watermark"].as_u64().unwrap() <= paused_watermark {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        h.audit()
            .iter()
            .map(|v| v["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["prepare"],
        "pause acknowledged before Coder spawn"
    );
    let snapshot = h.snapshot().await;
    assert_eq!(snapshot["runs"][0]["status"], "running");
    assert_eq!(snapshot["runs"][0]["stage"], "coder");
    assert_eq!(snapshot["execution"]["lane_run_id"], run["id"]);
    assert_eq!(snapshot["capabilities"]["run"], false);
    h.run(&task, "run-preflight-pause").await;
    assert_eq!(h.audit().len(), 1);
    h.command(
        "/execution/pause",
        json!({"paused":false}),
        "resume-after-preflight",
        200,
    )
    .await;
    assert_eq!(
        h.wait(&["succeeded", "unknown"]).await["status"],
        "succeeded"
    );
    assert_eq!(
        h.audit()
            .iter()
            .map(|v| v["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["prepare", "coder", "qa", "deliver"]
    );
    h.stop().await;
}

#[tokio::test]
async fn stop_during_preflight_wins_and_busy_second_task_never_dispatches() {
    let mut h = Harness::new(json!({"hold_preflight":true}), 15, true).await;
    h.unpause().await;
    let task = h.create("auto", "create-preflight-stop").await;
    let run = h.run(&task, "run-preflight-stop").await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !h.dir.join("preflight-waiting").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let second = h.create("auto", "create-second-busy").await;
    let rejected = h
        .command(
            &format!("/tasks/{}/run", second["id"].as_str().unwrap()),
            json!({"expected_revision":second["revision"]}),
            "second-cannot-acquire-lane",
            409,
        )
        .await;
    assert_eq!(rejected["error"], "active_run_exists");
    assert_eq!(h.snapshot().await["runs"].as_array().unwrap().len(), 1);
    assert_eq!(h.snapshot().await["capabilities"]["run"], false);
    h.command(
        "/execution/pause",
        json!({"paused":true}),
        "pause-before-preflight-stop",
        200,
    )
    .await;
    let path = format!("/runs/{}/cancel", run["id"].as_str().unwrap());
    let cancel = h.command(&path, json!({}), "stop-before-spawn", 202).await;
    assert_eq!(
        h.command(&path, json!({}), "stop-before-spawn", 202).await,
        cancel
    );
    std::fs::remove_file(h.dir.join("hold-preflight")).unwrap();
    let stopped = h.wait(&["cancelled", "unknown"]).await;
    assert_eq!(stopped["status"], "cancelled", "{stopped}");
    assert_eq!(
        h.audit()
            .iter()
            .map(|v| v["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["prepare", "cancel", "observe"]
    );
    let snapshot = h.snapshot().await;
    assert!(snapshot["execution"]["lane_run_id"].is_null());
    assert_eq!(snapshot["execution"]["paused"], true);
    assert!(
        snapshot["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["status"] == "ready")
    );
    h.stop().await;
}

#[tokio::test]
async fn negative_qa_releases_lane_without_any_publication_or_implicit_retry() {
    let mut h = Harness::new(json!({"qa_verdict_false":true}), 10, true).await;
    h.unpause().await;
    let task = h.create("auto", "create-negative-qa").await;
    let run = h.run(&task, "run-negative-qa").await;
    let failed = h.wait(&["failed", "unknown"]).await;
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["reason"], "qa_failed");
    assert_eq!(failed["receipt_count"], 3);
    assert_eq!(failed["evidence"]["qa_status"], "failed");
    assert!(failed["pull_request_url"].is_null());
    assert!(failed["merge_commit_sha"].is_null());
    let snapshot = h.snapshot().await;
    assert!(snapshot["execution"]["lane_run_id"].is_null());
    assert_eq!(snapshot["tasks"][0]["status"], "ready");
    assert!(snapshot["tasks"][0]["completed_at"].is_null());
    assert_eq!(h.run(&task, "run-negative-qa").await["id"], run["id"]);
    h.restart().await;
    assert_eq!(h.snapshot().await["runs"][0]["status"], "failed");
    assert_eq!(
        h.audit()
            .iter()
            .map(|v| v["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["prepare", "coder", "qa"]
    );
    h.stop().await;
}

#[tokio::test]
async fn stop_before_any_prepare_spawn_needs_only_proven_worker_cleanup() {
    let mut h = Harness::new(json!({}), 10, false).await;
    h.unpause().await;
    let task = h.create("auto", "create-never-started").await;
    let run = h.run(&task, "run-never-started").await;
    h.command(
        "/execution/pause",
        json!({"paused":true}),
        "pause-never-started",
        200,
    )
    .await;
    h.command(
        &format!("/runs/{}/cancel", run["id"].as_str().unwrap()),
        json!({}),
        "stop-never-started",
        202,
    )
    .await;
    h.engine = Some(h.app().start_execution());
    let cancelled = h.wait(&["cancelled", "unknown"]).await;
    assert_eq!(cancelled["status"], "cancelled");
    assert_eq!(cancelled["receipt_count"], 1);
    assert_eq!(
        h.audit()
            .iter()
            .map(|v| v["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["cancel"]
    );
    assert!(h.snapshot().await["execution"]["lane_run_id"].is_null());
    assert_eq!(h.snapshot().await["tasks"][0]["status"], "ready");
    assert!(
        !h.dir
            .join("work")
            .join(run["id"].as_str().unwrap())
            .exists()
    );
    h.stop().await;

    // A malformed prepare is still an actual attempted effect. Cleanup must
    // retain its observation requirement and cannot use the zero-spawn shortcut.
    let mut h = Harness::new(json!({"bad_receipts":["prepare","observe"]}), 10, true).await;
    h.unpause().await;
    let task = h.create("auto", "create-partial-prepare").await;
    let run = h.run(&task, "run-partial-prepare").await;
    h.wait(&["unknown"]).await;
    h.command(
        &format!("/runs/{}/cancel", run["id"].as_str().unwrap()),
        json!({}),
        "stop-partial-prepare",
        202,
    )
    .await;
    let uncertain = h.wait(&["unknown"]).await;
    assert_eq!(uncertain["status"], "unknown");
    assert_eq!(
        h.audit()
            .iter()
            .map(|v| v["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["prepare", "cancel", "observe"]
    );
    assert_eq!(h.snapshot().await["execution"]["lane_run_id"], run["id"]);
    h.stop().await;
}

#[tokio::test]
async fn bounded_repairs_keep_one_run_and_publish_only_the_final_independent_reviewed_head() {
    for rejects in [1, 2, 3] {
        let mut h = Harness::new(json!({"qa_rejections":rejects}), 20, true).await;
        let task = h.create("auto", &format!("create-repair-{rejects}")).await;
        h.unpause().await;
        let initial = h.run(&task, "repair-run").await;
        let result = h
            .wait(&[if rejects > 2 { "failed" } else { "succeeded" }])
            .await;
        assert_eq!(result["id"], initial["id"]);
        assert_eq!(result["deadline_ms"], initial["deadline_ms"]);
        assert_eq!(result["task_revision"], initial["task_revision"]);
        assert_eq!(result["max_qa_repairs"], 2);
        let audit = h.audit();
        let coders: Vec<_> = audit.iter().filter(|v| v["action"] == "coder").collect();
        let qas: Vec<_> = audit.iter().filter(|v| v["action"] == "qa").collect();
        assert_eq!(coders.len(), rejects.min(2) + 1);
        assert_eq!(qas.len(), rejects.min(2) + 1);
        assert_eq!(
            result["review_history"].as_array().unwrap().len(),
            qas.len()
        );
        assert!(result["session_history"].is_null());
        assert!(result["repair_context"].is_null());
        for i in 1..coders.len() {
            let ctx = &coders[i]["repair_context"];
            assert_eq!(ctx["cycle"], i);
            assert_eq!(ctx["reviewed_sha"], qas[i - 1]["head_sha"]);
            assert_eq!(ctx["qa_action_id"], qas[i - 1]["action_id"]);
            assert_eq!(ctx["findings"][0]["path"], "synthetic.txt");
            assert_eq!(coders[i]["head_sha"], qas[i - 1]["head_sha"]);
        }
        if rejects > 2 {
            assert_eq!(result["reason"], "qa_repairs_exhausted");
            assert!(!audit.iter().any(|v| v["action"] == "deliver"));
        } else {
            assert_eq!(audit.iter().filter(|v| v["action"] == "deliver").count(), 1);
            assert_eq!(qas.last().unwrap()["head_sha"], result["head_sha"]);
        }
        assert_eq!(h.run(&task, "repair-run").await["id"], result["id"]);
        assert_eq!(h.audit().len(), audit.len());
        h.stop().await;
    }
}

#[tokio::test]
async fn invalid_feedback_reused_identity_and_wrong_repair_ancestry_never_publish_or_retry() {
    for (mode, status, reason, coders) in [
        (
            json!({"qa_rejections":1,"empty_feedback":true}),
            "failed",
            "qa_feedback_missing",
            1,
        ),
        (
            json!({"qa_rejections":1,"malformed_feedback":true}),
            "unknown",
            "qa_findings_invalid",
            1,
        ),
        (
            json!({"qa_rejections":1,"reuse_coder_session":true}),
            "unknown",
            "worker_identity_reused",
            2,
        ),
        (
            json!({"qa_rejections":1,"reuse_qa_session":true}),
            "unknown",
            "worker_identity_reused",
            2,
        ),
        (
            json!({"qa_rejections":1,"non_descendant_repair":true}),
            "unknown",
            "git_verification_failed",
            2,
        ),
    ] {
        let mut h = Harness::new(mode, 20, true).await;
        let task = h.create("auto", "bad-repair-create").await;
        h.unpause().await;
        h.run(&task, "bad-repair-run").await;
        let result = h.wait(&[status]).await;
        assert_eq!(result["reason"], reason);
        assert_eq!(
            h.audit().iter().filter(|v| v["action"] == "coder").count(),
            coders
        );
        assert!(!h.audit().iter().any(|v| v["action"] == "deliver"));
        let count = h.audit().len();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(h.audit().len(), count);
        h.stop().await;
    }
}

#[tokio::test]
async fn pause_and_restart_hold_a_queued_repair_without_replaying_it() {
    let mut h = Harness::new(json!({"qa_rejections":1,"sleep_qa":0.4}), 20, true).await;
    let task = h.create("manual", "pause-repair-create").await;
    h.unpause().await;
    h.run(&task, "pause-repair-run").await;
    h.wait_action("qa").await;
    h.command(
        "/execution/pause",
        json!({"paused":true}),
        "pause-before-repair",
        200,
    )
    .await;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let r = h.snapshot().await["runs"][0].clone();
            if r["repair_count"] == 1 {
                assert_eq!(r["stage"], "coder");
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        h.audit().iter().filter(|v| v["action"] == "coder").count(),
        1
    );
    h.restart().await;
    assert_eq!(h.snapshot().await["runs"][0]["status"], "unknown");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        h.audit().iter().filter(|v| v["action"] == "coder").count(),
        1
    );
    h.stop().await;
}

#[tokio::test]
async fn deadline_during_rejected_qa_never_creates_a_repair_attempt() {
    let mut h = Harness::new(
        json!({"qa_rejections":2,"sleep_qa":3,"run_seconds":2}),
        20,
        true,
    )
    .await;
    let t = h.create("auto", "deadline-repair-create").await;
    h.unpause().await;
    h.run(&t, "deadline-repair-run").await;
    let r = h.wait(&["unknown"]).await;
    assert_eq!(r["repair_count"], 0);
    assert_eq!(
        h.audit().iter().filter(|v| v["action"] == "coder").count(),
        1
    );
    h.stop().await;
}

#[tokio::test]
async fn explicit_zero_one_and_legacy_missing_repair_bound_are_enforced() {
    for (limit, legacy) in [(0, false), (1, false), (2, true)] {
        let mut h =
            Harness::new(json!({"qa_rejections":2,"max_qa_repairs":limit}), 20, false).await;
        let t = h.create("auto", "bounded-config-create").await;
        h.unpause().await;
        let r = h.run(&t, "bounded-config-run").await;
        if legacy {
            // Synthetic migration fixture only: old persisted Run had no repair-bound field.
            let db = rusqlite::Connection::open(h.dir.join("board.sqlite")).unwrap();
            db.execute(
                "UPDATE runs SET document=json_remove(document,'$.max_qa_repairs') WHERE id=?1",
                [r["id"].as_str().unwrap()],
            )
            .unwrap();
        }
        h.engine = Some(h.app().start_execution());
        let result = h.wait(&["failed"]).await;
        assert_eq!(result["reason"], "qa_repairs_exhausted");
        assert_eq!(
            h.audit().iter().filter(|v| v["action"] == "coder").count(),
            if legacy { 1 } else { limit + 1 }
        );
        h.stop().await;
    }
}

#[tokio::test]
async fn unrelated_only_review_delivers_and_atomically_creates_mapped_manual_backlog_bugs() {
    let mut h = Harness::new(
        json!({"qa_out_of_scope":true,"out_scope_count":4}),
        20,
        true,
    )
    .await;
    let task=h.command("/tasks",json!({"title":"Scoped review","objective":"Implement only the requested task and related regressions.","repository":"fixture/repository","project":"Product label","workflow":"engineering","status":"ready","merge_policy":"auto"}),"scope-create",201).await["task"].clone();
    h.unpause().await;
    h.run(&task, "scope-run").await;
    let run = h.wait(&["succeeded"]).await;
    assert_eq!(run["repair_count"], 0);
    assert_eq!(run["review_history"][0]["findings"], json!([]));
    let snap = h.snapshot().await;
    let bugs: Vec<_> = snap["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["qa_source"].is_object())
        .cloned()
        .collect();
    assert_eq!(bugs.len(), 4);
    assert_eq!(snap["runs"].as_array().unwrap().len(), 1);
    for bug in &bugs {
        assert_eq!(bug["status"], "backlog");
        assert_eq!(bug["workflow"], "engineering");
        assert_eq!(bug["owner"], "Coder");
        assert_eq!(bug["repository"], task["repository"]);
        assert_eq!(bug["project"], task["project"]);
        assert_eq!(bug["automation_policy"], "manual");
        assert!(bug["automation_revision"].is_null());
        assert!(bug["ready_at"].is_null());
        assert_eq!(bug["qa_policy"], "required");
        assert_eq!(bug["merge_policy"], "auto");
        assert_eq!(
            bug["priority"],
            match bug["criticality"].as_str().unwrap() {
                "critical" => 1,
                "high" => 2,
                "medium" => 3,
                _ => 4,
            }
        );
        assert_eq!(bug["qa_source"]["task_id"], task["id"]);
        assert_eq!(bug["qa_source"]["run_id"], run["id"]);
        assert_eq!(bug["qa_source"]["reviewed_sha"], run["head_sha"]);
    }
    let bug = &bugs[0];
    let rejected = h
        .request(
            "PATCH",
            &format!("/tasks/{}", bug["id"].as_str().unwrap()),
            Some(json!({"expected_revision":1,"repository":"foreign/repository"})),
            Some("retarget-generated"),
            true,
        )
        .await;
    assert_eq!(rejected.status(), 409);
    // Synthetic metadata fixture: no project setup, host repository or provider.
    let project_id = uuid::Uuid::new_v4().to_string();
    let foreign = json!({"id":project_id,"repository":"foreign/repository","name":"Foreign project","repository_id":123,"default_branch":"main"});
    let fixture_db = rusqlite::Connection::open(h.dir.join("board.sqlite")).unwrap();
    fixture_db
        .execute(
            "INSERT INTO projects(id,repository,local_path,document) VALUES(?1,?2,?3,?4)",
            rusqlite::params![
                project_id,
                "foreign/repository",
                "synthetic-foreign-path",
                foreign.to_string()
            ],
        )
        .unwrap();
    drop(fixture_db);
    let rejected = h
        .request(
            "PATCH",
            &format!("/tasks/{}", bug["id"].as_str().unwrap()),
            Some(json!({"expected_revision":1,"project_id":project_id})),
            Some("rebind-generated"),
            true,
        )
        .await;
    assert_eq!(rejected.status(), 409);
    let audit = h.audit().len();
    h.run(&task, "scope-run").await;
    h.restart().await;
    assert_eq!(h.snapshot().await["tasks"].as_array().unwrap().len(), 5);
    assert_eq!(h.audit().len(), audit);
    h.stop().await;
}

#[tokio::test]
async fn mixed_review_repairs_only_blocking_findings_and_deduplicates_unrelated_reports() {
    let mut h = Harness::new(
        json!({"qa_out_of_scope":true,"qa_rejections":2,"sleep_qa":0.3}),
        20,
        true,
    )
    .await;
    let task = h.create("auto", "mixed-create").await;
    h.unpause().await;
    h.run(&task, "mixed-run").await;
    let triaged = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = h.snapshot().await;
            if let Some(bug) = snapshot["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["qa_source"].is_object())
            {
                break bug.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let response=h.request("PATCH",&format!("/tasks/{}",triaged["id"].as_str().unwrap()),Some(json!({"expected_revision":1,"title":"Human triage preserved","criticality":"medium","priority":4})),Some("human-triage"),true).await;
    assert_eq!(response.status(), 200);
    let run = h.wait(&["succeeded"]).await;
    assert_eq!(run["repair_count"], 2);
    let cycles = run["review_history"].as_array().unwrap();
    assert_eq!(cycles.len(), 3);
    let id = &cycles[0]["bug_task_ids"][0];
    assert!(id.is_string());
    for c in cycles {
        assert_eq!(&c["bug_task_ids"][0], id);
    }
    let snap = h.snapshot().await;
    assert_eq!(snap["tasks"].as_array().unwrap().len(), 2);
    let bug = snap["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == *id)
        .unwrap();
    assert_eq!(bug["criticality"], "medium");
    assert_eq!(bug["revision"], 2);
    assert_eq!(bug["priority"], 4);
    assert_eq!(bug["title"], "Human triage preserved");
    for coder in h
        .audit()
        .iter()
        .filter(|a| a["action"] == "coder" && a["cycle"] != 0)
    {
        assert_eq!(
            coder["repair_context"]["findings"][0]["path"],
            "synthetic.txt"
        );
        assert!(coder["repair_context"]["out_of_scope_bugs"].is_null());
    }
    assert_eq!(
        h.audit()
            .iter()
            .filter(|a| a["action"] == "deliver")
            .count(),
        1
    );
    h.stop().await;
}

#[tokio::test]
async fn invalid_unrelated_review_data_never_creates_tickets_or_repairs_or_publication() {
    for mode in [
        json!({"qa_out_of_scope":true,"out_scope_wrong_base":true}),
        json!({"qa_out_of_scope":true,"out_scope_injection":"repository"}),
        json!({"qa_out_of_scope":true,"out_scope_injection":"status"}),
        json!({"qa_out_of_scope":true,"out_scope_missing_evidence":true}),
        json!({"qa_out_of_scope":true,"out_scope_incomplete":true}),
        json!({"qa_out_of_scope":true,"qa_failed":true}),
    ] {
        let mut h = Harness::new(mode, 20, true).await;
        let task = h.create("auto", "invalid-scope-create").await;
        h.unpause().await;
        h.run(&task, "invalid-scope-run").await;
        let run = h.wait(&["unknown"]).await;
        assert_eq!(run["repair_count"], 0);
        assert_eq!(h.snapshot().await["tasks"].as_array().unwrap().len(), 1);
        assert!(!h.audit().iter().any(|a| a["action"] == "deliver"));
        h.stop().await;
    }
}

#[tokio::test]
async fn explicit_skip_qa_routes_verified_coder_directly_to_delivery_and_manual_merge_waits() {
    for merge in ["auto", "manual"] {
        let mut h = Harness::new(json!({}), 10, true).await;
        let task=h.command("/tasks",json!({"title":"Research outcome","objective":"Meaningful checks remain mandatory","repository":"fixture/repository","workflow":"engineering","task_type":"research","status":"ready","merge_policy":merge}),"research",201).await["task"].clone();
        assert_eq!(task["qa_policy"], "skip");
        h.unpause().await;
        h.run(&task, "run-skip-fixture").await;
        let run = h
            .wait(if merge == "auto" {
                &["succeeded"]
            } else {
                &["awaiting_merge"]
            })
            .await;
        assert_eq!(run["evidence"]["qa_status"], "skipped");
        assert!(run.get("coder_receipt").is_none());
        assert_eq!(run["receipt_count"], 3);
        assert_eq!(
            h.audit()
                .iter()
                .map(|v| v["action"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["prepare", "coder", "deliver"]
        );
        if merge == "manual" {
            h.restart().await;
            assert_eq!(
                h.wait(&["unknown"]).await["evidence"]["qa_status"],
                "skipped"
            );
            std::fs::write(
                h.dir.join("mode.json"),
                json!({"observed_merged":true}).to_string(),
            )
            .unwrap();
            h.command(
                &format!("/runs/{}/reconcile", run["id"].as_str().unwrap()),
                json!({}),
                "observe-merge",
                202,
            )
            .await;
            h.wait(&["succeeded"]).await;
        }
        h.stop().await;
    }
}
#[tokio::test]
async fn skipped_qa_never_authorizes_delivery_from_failed_coder_validation() {
    let mut h = Harness::new(json!({"coder_validation_failed":true}), 10, true).await;
    let task=h.command("/tasks",json!({"title":"Skipped QA","objective":"Cannot skip Coder checks","repository":"fixture/repository","workflow":"engineering","qa_policy":"skip","status":"ready"}),"create-skip",201).await["task"].clone();
    h.unpause().await;
    h.run(&task, "run-skip-fixture").await;
    h.wait(&["unknown"]).await;
    assert!(
        !h.audit()
            .iter()
            .any(|a| a["action"] == "qa" || a["action"] == "deliver")
    );
    h.stop().await;
}

#[tokio::test]
async fn qa_completion_transaction_failure_becomes_unknown_without_partial_bug_tickets_or_retry() {
    let mut h = Harness::new(
        json!({"qa_out_of_scope":true,"out_scope_count":4}),
        20,
        false,
    )
    .await;
    let task = h.create("auto", "capacity-source").await;
    // Synthetic legacy board near capacity; the third new bug exceeds the real
    // bound, after two tentative inserts in the same QA completion transaction.
    {
        let mut db = rusqlite::Connection::open(h.dir.join("board.sqlite")).unwrap();
        let tx = db.transaction().unwrap();
        for n in 0..9997 {
            let id = uuid::Uuid::new_v4().to_string();
            tx.execute("INSERT INTO tasks VALUES(?1,?2)",rusqlite::params![id,json!({"id":id,"title":format!("Synthetic filler {n}"),"status":"done","workflow":"manual"}).to_string()]).unwrap();
        }
        tx.commit().unwrap();
    }
    h.unpause().await;
    h.engine = Some(h.app().start_execution());
    h.run(&task, "capacity-run").await;
    let path = format!("/tasks/{}", task["id"].as_str().unwrap());
    let detail = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let v = body(h.request("GET", &path, None, None, true).await).await;
            if v["runs"][0]["status"] == "unknown" {
                break v;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("completed worker must not remain silently running after transaction rejection");
    assert_eq!(detail["runs"][0]["reason"], "completion_persistence_failed");
    assert_eq!(detail["task"]["status"], "active");
    assert_eq!(detail["runs"][0]["review_history"], json!([]));
    let db = rusqlite::Connection::open(h.dir.join("board.sqlite")).unwrap();
    let count: i64 = db
        .query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 9998);
    let attempt: String = db
        .query_row(
            "SELECT document FROM attempts ORDER BY rowid DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let attempt: Value = serde_json::from_str(&attempt).unwrap();
    assert_eq!(attempt["state"], "unknown");
    assert!(attempt["receipt"].is_null());
    let lane: Option<String> = db
        .query_row("SELECT lane_run_id FROM execution", [], |r| r.get(0))
        .unwrap();
    assert_eq!(lane.as_deref(), detail["runs"][0]["id"].as_str());
    drop(db);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        h.audit()
            .iter()
            .map(|a| a["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["prepare", "coder", "qa"]
    );
    h.command(
        &format!(
            "/runs/{}/reconcile",
            detail["runs"][0]["id"].as_str().unwrap()
        ),
        json!({}),
        "capacity-reconcile",
        202,
    )
    .await;
    let settled = h.wait(&["failed"]).await;
    assert_eq!(settled["reason"], "reconciled_without_delivery");
    let snapshot = h.snapshot().await;
    assert!(snapshot["execution"]["lane_run_id"].is_null());
    assert_eq!(snapshot["tasks"].as_array().unwrap().len(), 9998);
    assert!(
        !snapshot["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["qa_source"].is_object())
    );
    assert_eq!(
        h.audit()
            .iter()
            .map(|a| a["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["prepare", "coder", "qa", "observe"]
    );
    h.stop().await;
}

#[tokio::test]
async fn oversized_legacy_task_cannot_consume_a_run_and_can_be_reduced() {
    let mut h = Harness::new(json!({}), 20, false).await;
    let task = h.create("auto", "legacy-large-create").await;
    h.stop().await;
    {
        let db = rusqlite::Connection::open(h.dir.join("board.sqlite")).unwrap();
        let mut legacy = task.clone();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("dependency_blockers");
        legacy["objective"] = json!("o".repeat(8000));
        legacy["context"] = json!(vec!["c".repeat(1900); 16]);
        legacy["acceptance_criteria"] = json!(vec!["a".repeat(1900); 16]);
        assert!(legacy.to_string().len() > 65536);
        db.execute(
            "UPDATE tasks SET document=?1 WHERE id=?2",
            rusqlite::params![legacy.to_string(), task["id"].as_str()],
        )
        .unwrap();
    }
    h.restart().await;
    h.unpause().await;
    let path = format!("/tasks/{}", task["id"].as_str().unwrap());
    let refused = h
        .command(
            &format!("{path}/run"),
            json!({"expected_revision":task["revision"]}),
            "legacy-large-run",
            400,
        )
        .await;
    assert_eq!(refused["error"], "task_too_large");
    assert!(h.snapshot().await["runs"].as_array().unwrap().is_empty());
    assert!(h.audit().is_empty());
    let response=h.request("PATCH",&path,Some(json!({"expected_revision":task["revision"],"context":["Concise background"],"acceptance_criteria":["Focused acceptance case"]})),Some("legacy-large-reduce"),true).await;
    assert_eq!(response.status(), 200);
    let smaller = body(response).await["task"].clone();
    h.run(&smaller, "legacy-reduced-run").await;
    h.wait(&["succeeded"]).await;
    h.stop().await;
}
