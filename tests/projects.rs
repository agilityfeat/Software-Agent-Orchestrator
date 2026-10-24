//! Synthetic project setup + real local Git/process orchestration only.
#![cfg(unix)]
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};
use tower::ServiceExt;
use workboard_rs::{
    App,
    execution_config::{digest_file, now_ms},
    projects::Factory,
};
const KEY: &str = "project-fixture-key-0123456789abcdef";
fn write(path: &Path, bytes: impl AsRef<[u8]>, mode: u32) {
    std::fs::write(path, bytes).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}
fn dir(path: &Path) {
    std::fs::create_dir(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}
fn git(path: &Path, args: &[&str]) {
    assert!(
        std::process::Command::new("/usr/bin/git")
            .arg("-C")
            .arg(path)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    );
}
struct Harness {
    _root: tempfile::TempDir,
    root: PathBuf,
    app: App,
    cookie: String,
    csrf: String,
    engine: Option<tokio::task::JoinHandle<()>>,
}
impl Harness {
    async fn new(mode: Value) -> Self {
        let root = tempfile::tempdir().unwrap();
        let p = root.path().canonicalize().unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
        for n in ["work", "actions", "auth"] {
            dir(&p.join(n));
        }
        write(
            &p.join("driver.py"),
            include_str!("fixtures/driver.py"),
            0o700,
        );
        write(&p.join("mode.json"), mode.to_string(), 0o600);
        write(
            &p.join("gh.py"),
            "#!/usr/bin/python3\nimport sys,json,pathlib,subprocess\nr=sys.argv[-1].removeprefix('repos/')\nif '/contents/' in r:\n name=r.split('/')[1];file=r.split('/contents/')[1].split('?')[0];sha=subprocess.check_output(['/usr/bin/git','-C',str(pathlib.Path(__file__).resolve().parent/name),'hash-object','--no-filters','--',file],text=True).strip();print(json.dumps({'type':'file','sha':sha}))\nelse:print(json.dumps({'id':123 if r.endswith('/a') else 456,'full_name':r,'private':not r.endswith('/public'),'archived':False,'default_branch':'main','permissions':{'push':True}}))\n",
            0o700,
        );
        let program = |name: &Path| json!({"executable":name,"sha256":digest_file(name).unwrap(),"credential_env_names":[]});
        let driver = program(&p.join("driver.py"));
        write(
            &p.join("git.py"),
            include_str!("fixtures/project-git.py"),
            0o700,
        );
        let actualgit = program(&p.join("git.py"));
        let mut adapter = json!({"repository":"fixture/template","state_root":p.join("actions"),"auth_home":p.join("auth"),"docker":driver,"git":actualgit,"gh":program(&p.join("gh.py")),"docker_host":"unix:///synthetic-only.sock","worker_image":format!("sha256:{}","1".repeat(64)),"worker_network":"2".repeat(64),"proxy_container":"3".repeat(64),"proxy_image":format!("sha256:{}","4".repeat(64)),"proxy_ip":"192.0.2.2","proxy_port":3128,"proxy_policy_sha256":"5".repeat(64),"proxy_allowed_hosts":["chatgpt.com"],"model":null,"cpus":1,"memory_mb":256});
        if mode["native"] == true {
            adapter["native_agent"] = json!({"codex":driver,"projects_root":p,"catalog_path":p.join("catalog.json"),"path":"/usr/bin:/bin","read_roots":[],"agents":[{"id":"coder","label":"Coder","instructions":"Implement"},{"id":"specialist","label":"Specialist","instructions":"Reviewed specialty"},{"id":"qa","label":"QA","instructions":"Independent"}],"default_coder":"coder","qa_agent":"qa"});
        }
        let setup = json!({"schema_version":1,"planning_enabled":mode["planning_enabled"]==true,"max_runs_per_project":100,"runtime":{"schema_version":1,"containment_contract":"reviewed-launcher-v1","expires_at_ms":now_ms()+3600000,"work_root":p.join("work"),"worker":driver,"publisher":driver,"git":actualgit,"max_run_seconds":60,"max_stage_seconds":mode["plan_stage_seconds"].as_u64().unwrap_or(20),"max_output_bytes":8192,"adapter_policy":adapter}});
        write(&p.join("setup.json"), setup.to_string(), 0o600);
        let factory = Factory::load(
            &p.join("setup.json"),
            KEY.as_bytes(),
            &digest_file(&p.join("setup.json")).unwrap(),
        )
        .unwrap();
        let app = App::open_projects(&p.join("board.sqlite"), KEY, 4311, factory).unwrap();
        let mut h = Self {
            _root: root,
            root: p,
            app,
            cookie: String::new(),
            csrf: String::new(),
            engine: None,
        };
        let r = h
            .request("POST", "/session", json!({"key":KEY}), None, false)
            .await;
        h.cookie = r.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .into();
        h.csrf = body(r).await["csrf_token"].as_str().unwrap().into();
        h
    }
    async fn request(
        &self,
        method: &str,
        path: &str,
        data: Value,
        key: Option<&str>,
        auth: bool,
    ) -> axum::response::Response {
        let mut r = Request::builder()
            .method(method)
            .uri(format!("/api/workboard/v1{path}"))
            .header("host", "127.0.0.1:4311");
        if method != "GET" {
            r = r
                .header("origin", "http://127.0.0.1:4311")
                .header("content-type", "application/json");
        }
        if auth {
            r = r
                .header("cookie", &self.cookie)
                .header("x-csrf-token", &self.csrf);
        }
        if let Some(k) = key {
            r = r.header("idempotency-key", k);
        }
        self.app
            .router()
            .oneshot(r.body(Body::from(data.to_string())).unwrap())
            .await
            .unwrap()
    }
    async fn call(&self, method: &str, path: &str, data: Value, key: &str, status: u16) -> Value {
        let r = self.request(method, path, data, Some(key), true).await;
        let actual = r.status();
        let v = body(r).await;
        assert_eq!(actual.as_u16(), status, "{path}: {v}");
        v
    }
    async fn snapshot(&self) -> Value {
        body(
            self.request("GET", "/snapshot", Value::Null, None, true)
                .await,
        )
        .await
    }
    fn repo(&self, name: &str) -> PathBuf {
        let p = self.root.join(name);
        dir(&p);
        dir(&p.join("scripts"));
        dir(&p.join("test"));
        write(&p.join("package.json"),json!({"name":name,"type":"module","scripts":{"test":"node --test test/*.test.js","build":"node scripts/build.js"}}).to_string(),0o600);
        write(
            &p.join("scripts/build.js"),
            "// synthetic profile discovery only\n",
            0o600,
        );
        git(&p, &["init", "--initial-branch=main"]);
        git(
            &p,
            &[
                "remote",
                "add",
                "origin",
                &format!("https://github.com/fixture/{name}.git"),
            ],
        );
        git(&p, &["add", "."]);
        git(
            &p,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-m",
                "Synthetic baseline",
            ],
        );
        p
    }
    async fn project(&self, name: &str) -> Value {
        let path = self.repo(name);
        self.call(
            "POST",
            "/projects",
            json!({"local_path":path}),
            &format!("create-project-{name}"),
            201,
        )
        .await["project"]
            .clone()
    }
    async fn task(&self, p: &Value, title: &str, status: &str, priority: u32) -> Value {
        self.call("POST","/tasks",json!({"project_id":p["id"],"workflow":"engineering","title":title,"objective":"Synthetic process fixture only","acceptance_criteria":["deterministic local pipeline"],"status":status,"priority":priority,"automation_policy":"ready_to_done","qa_policy":"required","merge_policy":"auto"}),&format!("create-task-{title}"),201).await["task"].clone()
    }
    async fn start_project(&self, p: &Value, key: &str) {
        self.call(
            "PATCH",
            &format!("/projects/{}", p["id"].as_str().unwrap()),
            json!({"expected_revision":p["revision"],"paused":false}),
            key,
            200,
        )
        .await;
    }
    async fn start(&mut self) {
        self.call(
            "POST",
            "/execution/pause",
            json!({"paused":false}),
            "resume-fixture-global",
            200,
        )
        .await;
        self.engine = Some(self.app.start_execution());
    }
    async fn wait(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        let result = tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                let s = self.snapshot().await;
                if predicate(&s) {
                    return s;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        result.unwrap_or_else(|_| panic!("synthetic pipeline timeout; audit={:?}", self.audit()))
    }
    async fn stop(&mut self) {
        self.app.shutdown();
        if let Some(e) = self.engine.take() {
            tokio::time::timeout(Duration::from_secs(5), e)
                .await
                .unwrap()
                .unwrap();
        }
    }
    fn audit(&self) -> Vec<Value> {
        std::fs::read_to_string(self.root.join("audit.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    }
}
async fn body(r: axum::response::Response) -> Value {
    serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
#[tokio::test]
async fn real_setup_binds_readonly_project_and_parallel_queues_serialize_each_project() {
    let mut h = Harness::new(json!({"sleep_coder":0.4})).await;
    let a = h.project("a").await;
    let b = h.project("b").await;
    assert!(a.get("approval_path").is_none());
    assert_eq!(a["default_branch"], "main");
    assert_eq!(a["profile"]["id"], "node-builtin-review-v1");
    let old = h.task(&a, "older-backlog", "backlog", 3).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    let first = h.task(&a, "ready-first", "ready", 3).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    let late = h
        .call(
            "PATCH",
            &format!("/tasks/{}", old["id"].as_str().unwrap()),
            json!({"expected_revision":1,"status":"ready","automation_policy":"ready_to_done"}),
            "promote-older-backlog",
            200,
        )
        .await["task"]
        .clone();
    let urgent = h.task(&a, "urgent-a", "ready", 1).await;
    let other = h.task(&b, "parallel-b", "ready", 3).await;
    h.start_project(&a, "start-project-a").await;
    h.start_project(&b, "start-project-b").await;
    h.start().await;
    let s = h
        .wait(|s| {
            s["runs"].as_array().unwrap().len() == 4
                && s["runs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|r| r["status"] == "succeeded")
        })
        .await;
    let a_runs: Vec<_> = s["runs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["project_id"] == a["id"])
        .collect();
    assert_eq!(
        a_runs
            .iter()
            .map(|r| r["task_id"].clone())
            .collect::<Vec<_>>(),
        vec![
            urgent["id"].clone(),
            first["id"].clone(),
            late["id"].clone()
        ]
    );
    let audit = h.audit();
    let prepare_ids: Vec<_> = audit
        .iter()
        .filter(|r| r["action"] == "prepare")
        .map(|r| r["run_id"].clone())
        .collect();
    let first_a = &a_runs[0]["id"];
    let b_run = s["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["task_id"] == other["id"])
        .unwrap();
    assert!(prepare_ids[..2].contains(first_a) && prepare_ids[..2].contains(&b_run["id"]));
    let first_deliver = audit
        .iter()
        .position(|r| r["action"] == "deliver" && r["run_id"] == *first_a)
        .unwrap();
    let next_prepare = audit
        .iter()
        .position(|r| r["action"] == "prepare" && r["run_id"] == a_runs[1]["id"])
        .unwrap();
    assert!(first_deliver < next_prepare);
    assert!(
        s["projects"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["lane_run_id"].is_null() && p["paused"] == false)
    );
    // Alias setup is refused before any task or worker can dispatch twice.
    let alias = h.repo("alias");
    git(
        &alias,
        &[
            "remote",
            "set-url",
            "origin",
            "https://github.com/fixture/a.git",
        ],
    );
    h.call(
        "POST",
        "/projects",
        json!({"local_path":alias}),
        "alias-same-repository",
        409,
    )
    .await;
    let renamed = h.repo("renamed-b");
    h.call(
        "POST",
        "/projects",
        json!({"local_path":renamed}),
        "alias-same-github-id",
        409,
    )
    .await;
    h.stop().await;
}
#[tokio::test]
async fn negative_qa_pauses_only_affected_project_and_never_retries() {
    let mut h = Harness::new(json!({"mode_by_repository":{"fixture/a":{"qa_failed":true}}})).await;
    let a = h.project("a").await;
    let b = h.project("b").await;
    h.task(&a, "fails-a", "ready", 3).await;
    h.task(&a, "later-a", "ready", 3).await;
    h.task(&b, "passes-b", "ready", 3).await;
    h.task(&b, "later-b", "ready", 3).await;
    h.start_project(&a, "start-failing-project").await;
    h.start_project(&b, "start-other-project").await;
    h.start().await;
    let s = h
        .wait(|s| {
            s["runs"].as_array().unwrap().len() == 3
                && s["runs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|r| ["failed", "succeeded"].contains(&r["status"].as_str().unwrap_or("")))
        })
        .await;
    let projects = s["projects"].as_array().unwrap();
    assert_eq!(
        projects.iter().find(|p| p["id"] == a["id"]).unwrap()["paused"],
        true
    );
    assert_eq!(
        projects.iter().find(|p| p["id"] == b["id"]).unwrap()["paused"],
        false
    );
    assert_eq!(
        s["runs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["project_id"] == a["id"])
            .count(),
        1
    );
    h.stop().await;
}

#[tokio::test]
async fn project_pause_defers_next_stage_and_resume_advances_the_same_run() {
    let mut h = Harness::new(json!({"mode_by_repository":{"fixture/a":{"sleep_coder":0.5}}})).await;
    let a = h.project("a").await;
    h.task(&a, "pause-between-stages", "ready", 3).await;
    h.start_project(&a, "start-pause-project").await;
    h.start().await;
    h.wait(|_| h.audit().iter().any(|v| v["action"] == "coder"))
        .await;
    let p = h.snapshot().await["projects"][0].clone();
    let paused = h
        .call(
            "PATCH",
            &format!("/projects/{}", a["id"].as_str().unwrap()),
            json!({"expected_revision":p["revision"],"paused":true}),
            "pause-during-coder",
            200,
        )
        .await["project"]
        .clone();
    tokio::time::sleep(Duration::from_millis(750)).await;
    assert!(!h.audit().iter().any(|v| v["action"] == "qa"));
    let before = h.snapshot().await;
    assert_eq!(before["runs"].as_array().unwrap().len(), 1);
    h.start_project(&paused, "resume-same-project-run").await;
    let after = h.wait(|s| s["runs"][0]["status"] == "succeeded").await;
    assert_eq!(after["runs"][0]["id"], before["runs"][0]["id"]);
    assert_eq!(
        h.audit().iter().filter(|v| v["action"] == "coder").count(),
        1
    );
    h.stop().await;
}
#[tokio::test]
async fn setup_replay_dirty_folder_and_reconnect_are_bounded_without_product_mutations() {
    let mut h = Harness::new(json!({})).await;
    let a = h.project("a").await;
    let original = std::fs::read(h.root.join("a/package.json")).unwrap();
    let replay = h
        .call(
            "POST",
            "/projects",
            json!({"local_path":h.root.join("a")}),
            "create-project-a",
            201,
        )
        .await;
    assert_eq!(replay["project"], a);
    let dirty = h.repo("dirty");
    write(
        &dirty.join("uncommitted.txt"),
        "local work stays untouched",
        0o600,
    );
    h.call(
        "POST",
        "/projects",
        json!({"local_path":dirty}),
        "reject-dirty-folder",
        409,
    )
    .await;
    let filtered = h.repo("filtered");
    git(
        &filtered,
        &[
            "config",
            "filter.bad.clean",
            "touch /tmp/must-not-execute-workboard-filter",
        ],
    );
    h.call(
        "POST",
        "/projects",
        json!({"local_path":filtered}),
        "reject-clean-filter",
        400,
    )
    .await;
    let task = h.task(&a, "reconnect-revokes", "ready", 3).await;
    let refreshed = h
        .call(
            "PATCH",
            &format!("/projects/{}", a["id"].as_str().unwrap()),
            json!({"expected_revision":1,"reconnect":true}),
            "refresh-detected-checks",
            200,
        )
        .await["project"]
        .clone();
    assert_eq!(refreshed["id"], a["id"]);
    assert_eq!(refreshed["revision"], 2);
    assert_eq!(refreshed["paused"], true);
    let snap = h.snapshot().await;
    let current = snap["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == task["id"])
        .unwrap();
    assert_eq!(current["automation_policy"], "manual");
    assert!(current["automation_revision"].is_null());
    assert_eq!(
        std::fs::read(h.root.join("a/package.json")).unwrap(),
        original
    );
    assert!(h.audit().is_empty());
    h.stop().await;
}

#[tokio::test]
async fn restart_unknown_holds_only_its_project_and_other_project_still_completes() {
    let mut h = Harness::new(json!({"mode_by_repository":{"fixture/a":{"sleep_coder":3}}})).await;
    let a = h.project("a").await;
    let b = h.project("b").await;
    h.task(&a, "restart-active-a", "ready", 3).await;
    h.start_project(&a, "start-restart-a").await;
    h.start().await;
    h.wait(|_| h.audit().iter().any(|v| v["action"] == "coder"))
        .await;
    h.stop().await;
    let factory = Factory::load(
        &h.root.join("setup.json"),
        KEY.as_bytes(),
        &digest_file(&h.root.join("setup.json")).unwrap(),
    )
    .unwrap();
    let dummy = App::open(&h.root.join("unused-fixture.sqlite"), KEY, 4311).unwrap();
    let old = std::mem::replace(&mut h.app, dummy);
    drop(old);
    h.app = App::open_projects(&h.root.join("board.sqlite"), KEY, 4311, factory).unwrap();
    let r = h
        .request("POST", "/session", json!({"key":KEY}), None, false)
        .await;
    h.cookie = r.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .into();
    h.csrf = body(r).await["csrf_token"].as_str().unwrap().into();
    h.engine = Some(h.app.start_execution());
    let snap = h.snapshot().await;
    assert_eq!(snap["runs"][0]["status"], "unknown");
    let current_a = snap["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == a["id"])
        .unwrap();
    assert_eq!(current_a["paused"], true);
    h.call(
        "PATCH",
        &format!("/projects/{}", a["id"].as_str().unwrap()),
        json!({"expected_revision":current_a["revision"],"paused":false}),
        "cannot-resume-unknown-a",
        409,
    )
    .await;
    h.task(&b, "restart-other-b", "ready", 3).await;
    h.start_project(&b, "start-b-after-restart").await;
    let snap = h
        .wait(|s| {
            s["runs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["project_id"] == b["id"] && r["status"] == "succeeded")
        })
        .await;
    assert_eq!(
        snap["runs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["project_id"] == a["id"])
            .count(),
        1
    );
    assert_eq!(
        h.audit().iter().filter(|r| r["action"] == "coder").count(),
        2
    );
    h.stop().await;
}

#[tokio::test]
async fn one_project_approval_drift_is_reported_without_starving_other_projects() {
    let mut h = Harness::new(json!({})).await;
    let a = h.project("a").await;
    let b = h.project("b").await;
    h.task(&a, "bad-approval-a", "ready", 1).await;
    h.task(&b, "healthy-approval-b", "ready", 3).await;
    h.start_project(&a, "start-bad-config-a").await;
    h.start_project(&b, "start-healthy-config-b").await;
    // Only this isolated fixture's A approval bytes drift; no running action or
    // production approval is changed, and B keeps its original signed runtime.
    let configs =
        std::fs::read_dir(h.root.join("actions").join(a["id"].as_str().unwrap())).unwrap();
    let config = configs
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().starts_with("execution-"))
        .unwrap()
        .path();
    let mut bytes = std::fs::read(&config).unwrap();
    bytes.push(b' ');
    write(&config, bytes, 0o600);
    h.start().await;
    let s = h
        .wait(|s| {
            s["runs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["project_id"] == b["id"] && r["status"] == "succeeded")
        })
        .await;
    assert_eq!(s["runs"].as_array().unwrap().len(), 1);
    let a_state = s["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == a["id"])
        .unwrap();
    assert_eq!(a_state["paused"], true);
    assert!(a_state["failure"].is_string());
    h.stop().await;
}

#[tokio::test]
async fn unsupported_or_public_project_cannot_claim_a_runtime_and_mixed_ownership_is_rejected() {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut h = Harness::new(json!({})).await;
    let public = h.repo("public");
    h.call(
        "POST",
        "/projects",
        json!({"local_path":public}),
        "reject-public-project",
        400,
    )
    .await;
    let unsupported = h.repo("unsupported");
    write(&unsupported.join("package.json"),json!({"type":"module","scripts":{"test":"arbitrary task command","build":"node scripts/build.js"}}).to_string(),0o600);
    git(&unsupported, &["add", "package.json"]);
    git(
        &unsupported,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            "Unsupported synthetic profile",
        ],
    );
    h.call(
        "POST",
        "/projects",
        json!({"local_path":unsupported}),
        "reject-unsupported-profile",
        400,
    )
    .await;
    assert!(
        h.snapshot().await["projects"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(h.audit().is_empty());
    let setup: Value =
        serde_json::from_slice(&std::fs::read(h.root.join("setup.json")).unwrap()).unwrap();
    let mut payload = setup["runtime"].clone();
    payload["ownership"] = json!({"schema_version":1,"database_path":h.root.join("unopened-owner.sqlite"),"record_id":uuid::Uuid::new_v4().to_string(),"repository":"fixture/a","epoch":1});
    let mut mac = Hmac::<Sha256>::new_from_slice(KEY.as_bytes()).unwrap();
    mac.update(payload.to_string().as_bytes());
    let config = h.root.join("legacy-owned.json");
    write(
        &config,
        json!({"payload":payload,"signature":format!("{:x}",mac.finalize().into_bytes())})
            .to_string(),
        0o600,
    );
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_workboard-rs"))
        .env_clear()
        .env("WORKBOARD_DB", h.root.join("mixed.sqlite"))
        .env("WORKBOARD_PORT", port.to_string())
        .env("WORKBOARD_OPERATOR_KEY", KEY)
        .env("WORKBOARD_PROJECT_SETUP", h.root.join("setup.json"))
        .env(
            "WORKBOARD_PROJECT_SETUP_SHA256",
            digest_file(&h.root.join("setup.json")).unwrap(),
        )
        .env("WORKBOARD_PROJECT_SETUP_KEY", KEY)
        .env("WORKBOARD_EXECUTION_CONFIG", &config)
        .env(
            "WORKBOARD_EXECUTION_APPROVAL_SHA256",
            digest_file(&config).unwrap(),
        )
        .env("WORKBOARD_EXECUTION_VERIFY_KEY", KEY)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("project_setup_mixed_ownership"));
    assert!(!h.root.join("unopened-owner.sqlite").exists());
    h.stop().await;
}

#[tokio::test]
async fn tickets_discover_while_paused_and_repository_labels_share_a_serial_lane() {
    let mut h = Harness::new(json!({"delay_coder":0.15})).await;
    let source = h.repo("a");
    let before = digest_file(&source.join("package.json")).unwrap();
    h.engine = Some(h.app.start_execution());
    let first=h.call("POST","/tasks",json!({"project":"First label","repository":"fixture/a","title":"first","objective":"Synthetic queue","acceptance_criteria":["Done"]}),"ticket-first",201).await["task"].clone();
    assert!(first["project_id"].is_null());
    assert_eq!(first["workflow"], "engineering");
    assert_eq!(first["qa_policy"], "required");
    assert_eq!(first["merge_policy"], "auto");
    h.call(
        "PATCH",
        &format!("/tasks/{}", first["id"].as_str().unwrap()),
        json!({"expected_revision":first["revision"],"status":"ready"}),
        "ticket-ready",
        200,
    )
    .await;
    let configured = h
        .wait(|s| {
            s["projects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["runtime_status"] == "configured")
        })
        .await;
    assert_eq!(configured["runs"].as_array().unwrap().len(), 0);
    assert_eq!(configured["projects"].as_array().unwrap().len(), 1);
    assert_eq!(digest_file(&source.join("package.json")).unwrap(), before);
    let second=h.call("POST","/tasks",json!({"project":"Different label","repository":"FIXTURE/a","title":"second","objective":"Same repository queue","acceptance_criteria":["Done"],"status":"ready"}),"ticket-second",201).await["task"].clone();
    h.call(
        "POST",
        "/execution/pause",
        json!({"paused":false}),
        "ticket-resume",
        200,
    )
    .await;
    let done = h
        .wait(|s| {
            s["runs"].as_array().unwrap().len() == 2
                && s["runs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|r| r["status"] == "succeeded")
        })
        .await;
    assert_eq!(done["projects"].as_array().unwrap().len(), 1);
    for task in done["tasks"].as_array().unwrap() {
        if task["id"] == first["id"] {
            assert_eq!(task["project"], "First label");
        }
        if task["id"] == second["id"] {
            assert_eq!(task["project"], "Different label");
        }
    }
    let runs = done["runs"].as_array().unwrap();
    let a = runs.iter().find(|r| r["task_id"] == first["id"]).unwrap();
    let b = runs.iter().find(|r| r["task_id"] == second["id"]).unwrap();
    assert!(a["updated_at"].as_str().unwrap() <= b["created_at"].as_str().unwrap());
    h.stop().await;
}

#[tokio::test]
async fn discovery_failure_isolated_and_retry_is_async_without_local_folder() {
    let mut h = Harness::new(json!({})).await;
    h.repo("a");
    h.repo("b");
    write(&h.root.join("a/package.json"), "{}", 0o600);
    git(&h.root.join("a"), &["add", "package.json"]);
    git(
        &h.root.join("a"),
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            "unsupported synthetic settings",
        ],
    );
    for name in ["a", "b"] {
        h.call("POST","/tasks",json!({"project":name,"repository":format!("fixture/{name}"),"title":name,"objective":"Synthetic discovery isolation","acceptance_criteria":["Done"],"status":"ready"}),&format!("ticket-{name}"),201).await;
    }
    h.start().await;
    let result = h
        .wait(|s| {
            s["runs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["repository"] == "fixture/b" && r["status"] == "succeeded")
                && s["projects"].as_array().unwrap().iter().any(|p| {
                    p["repository"] == "fixture/a" && p["failure"] == "project_profile_unsupported"
                })
        })
        .await;
    assert_eq!(result["runs"].as_array().unwrap().len(), 1);
    let a = result["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["repository"] == "fixture/a")
        .unwrap();
    write(&h.root.join("a/package.json"),json!({"name":"a","type":"module","scripts":{"test":"node --test test/*.test.js","build":"node scripts/build.js"}}).to_string(),0o600);
    git(&h.root.join("a"), &["add", "package.json"]);
    git(
        &h.root.join("a"),
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            "restore synthetic settings",
        ],
    );
    let retry = h
        .call(
            "PATCH",
            &format!("/projects/{}", a["id"].as_str().unwrap()),
            json!({"expected_revision":a["revision"],"reconnect":true}),
            "retry-repository",
            200,
        )
        .await;
    assert_eq!(retry["project"]["runtime_status"], "pending");
    h.wait(|s| {
        s["runs"].as_array().unwrap().len() == 2
            && s["runs"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["status"] == "succeeded")
    })
    .await;
    h.stop().await;
}

#[tokio::test]
async fn preparation_shutdown_is_bounded_and_restart_exposes_interrupted_setup_without_a_run() {
    let mut h = Harness::new(json!({})).await;
    h.repo("a");
    write(
        &h.root.join("slow-clone"),
        "synthetic preparation interruption",
        0o600,
    );
    h.call("POST","/tasks",json!({"repository":"fixture/a","project":"label","title":"prepare only","objective":"No worker launch","acceptance_criteria":["restart is explicit"],"status":"ready"}),"interrupted-ticket",201).await;
    h.engine = Some(h.app.start_execution());
    h.wait(|_| h.root.join("clone-started").exists()).await;
    let e = h.engine.take().unwrap();
    e.abort();
    let _ = e.await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let factory = Factory::load(
        &h.root.join("setup.json"),
        KEY.as_bytes(),
        &digest_file(&h.root.join("setup.json")).unwrap(),
    )
    .unwrap();
    let dummy = App::open(&h.root.join("dummy.sqlite"), KEY, 4311).unwrap();
    let old = std::mem::replace(&mut h.app, dummy);
    drop(old);
    h.app = App::open_projects(&h.root.join("board.sqlite"), KEY, 4311, factory).unwrap();
    let r = h
        .request("POST", "/session", json!({"key":KEY}), None, false)
        .await;
    h.cookie = r.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .into();
    h.csrf = body(r).await["csrf_token"].as_str().unwrap().into();
    let s = h.snapshot().await;
    assert_eq!(s["runs"].as_array().unwrap().len(), 0);
    assert_eq!(s["projects"][0]["paused"], true);
    assert_eq!(s["projects"][0]["failure"], "project_discovery_interrupted");
    std::fs::remove_file(h.root.join("slow-clone")).unwrap();
    let p = &s["projects"][0];
    h.call(
        "PATCH",
        &format!("/projects/{}", p["id"].as_str().unwrap()),
        json!({"expected_revision":p["revision"],"reconnect":true}),
        "retry-interrupted",
        200,
    )
    .await;
    h.engine = Some(h.app.start_execution());
    h.wait(|s| s["projects"][0]["runtime_status"] == "configured")
        .await;
    assert!(h.audit().is_empty());
    h.stop().await;
}

#[tokio::test]
async fn remote_build_script_symlink_is_rejected_without_reading_outside_clone() {
    let mut h = Harness::new(json!({})).await;
    let source = h.repo("a");
    let outside = h.root.join("outside-sentinel");
    write(&outside, "outside must not become a profile pin", 0o600);
    std::fs::remove_file(source.join("scripts/build.js")).unwrap();
    std::os::unix::fs::symlink(&outside, source.join("scripts/build.js")).unwrap();
    git(&source, &["add", "scripts/build.js"]);
    git(
        &source,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            "synthetic symlink",
        ],
    );
    h.call("POST","/tasks",json!({"repository":"fixture/a","title":"symlink","objective":"Bound discovery","acceptance_criteria":["reject"],"status":"ready"}),"symlink-ticket",201).await;
    h.engine = Some(h.app.start_execution());
    let s = h
        .wait(|s| {
            s["projects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["failure"] == "project_profile_unsupported")
        })
        .await;
    assert!(s["runs"].as_array().unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(outside).unwrap(),
        "outside must not become a profile pin"
    );
    h.stop().await;
}

#[tokio::test]
async fn manual_delivery_ticket_prepares_checks_without_implicit_worker_admission() {
    let mut h = Harness::new(json!({})).await;
    h.repo("a");
    let task=h.call("POST","/tasks",json!({"workflow":"engineering","repository":"fixture/a","project":"Manual delivery","merge_policy":"manual","automation_policy":"manual","title":"manual ticket","objective":"Respect merge veto","acceptance_criteria":["No automatic merge"],"status":"ready"}),"manual-delivery-ticket",201).await["task"].clone();
    assert_eq!(task["automation_policy"], "manual");
    h.engine = Some(h.app.start_execution());
    h.wait(|s| {
        s["projects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["runtime_status"] == "configured")
    })
    .await;
    h.call(
        "POST",
        "/execution/pause",
        json!({"paused":false}),
        "resume-manual-ticket",
        200,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(h.snapshot().await["runs"].as_array().unwrap().is_empty());
    h.call(
        "POST",
        &format!("/tasks/{}/run", task["id"].as_str().unwrap()),
        json!({"expected_revision":task["revision"]}),
        "explicit-manual-run",
        202,
    )
    .await;
    let s = h
        .wait(|s| {
            s["runs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["status"] == "awaiting_merge")
        })
        .await;
    assert_eq!(s["tasks"][0]["status"], "active"); // UI projects awaiting_merge as Review, lane remains owned.
    h.stop().await;
}

#[tokio::test]
async fn automatic_ready_processing_reaches_manual_merge_review_without_explicit_run() {
    let mut h = Harness::new(json!({})).await;
    h.repo("a");
    let t=h.call("POST","/tasks",json!({"repository":"fixture/a","project":"label","merge_policy":"manual","title":"automatic manual merge","objective":"Process automatically but never merge","acceptance_criteria":["Wait for human merge"],"status":"ready"}),"auto-manual-delivery",201).await["task"].clone();
    assert_eq!(t["automation_policy"], "ready_to_done");
    assert_eq!(t["merge_policy"], "manual");
    h.start().await;
    let s = h
        .wait(|s| {
            s["runs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["status"] == "awaiting_merge")
        })
        .await;
    assert_eq!(s["runs"].as_array().unwrap().len(), 1);
    assert!(!s["projects"][0]["lane_run_id"].is_null());
    assert!(s["tasks"][0]["completed_at"].is_null());
    h.stop().await;
}

#[tokio::test]
async fn exhausted_repairs_pause_only_their_repository_while_other_queue_completes() {
    let mut h = Harness::new(json!({"mode_by_repository":{"fixture/a":{"qa_rejections":3}}})).await;
    let a = h.project("a").await;
    let b = h.project("b").await;
    h.task(&a, "repair-a", "ready", 3).await;
    h.task(&b, "healthy-b1", "ready", 3).await;
    h.task(&b, "healthy-b2", "ready", 3).await;
    h.start_project(&a, "start-repair-a").await;
    h.start_project(&b, "start-healthy-b").await;
    h.start().await;
    let s = h
        .wait(|s| {
            s["runs"].as_array().unwrap().len() == 3
                && s["runs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|r| ["succeeded", "failed"].contains(&r["status"].as_str().unwrap_or("")))
        })
        .await;
    let projects = s["projects"].as_array().unwrap();
    assert_eq!(
        projects.iter().find(|p| p["id"] == a["id"]).unwrap()["paused"],
        true
    );
    assert_eq!(
        projects.iter().find(|p| p["id"] == b["id"]).unwrap()["paused"],
        false
    );
    let failed = s["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["repository"] == "fixture/a")
        .unwrap();
    assert_eq!(failed["reason"], "qa_repairs_exhausted");
    assert_eq!(failed["repair_count"], 2);
    h.stop().await;
}

#[tokio::test]
async fn main_plan_review_acceptance_then_ready_uses_the_existing_automatic_queue() {
    let mut h = Harness::new(json!({"planning_enabled":true})).await;
    h.repo("a");
    let goal = json!({"goal":"Deliver a focused improvement","context":"Preserve the documented constraints without external writes.","project":"Fixture product","repository":"fixture/a"});
    h.call("POST", "/plans", goal.clone(), "paused-main-plan", 409)
        .await;
    h.start().await;
    let plan = h
        .call("POST", "/plans", goal.clone(), "generate-main-plan", 202)
        .await["plan"]
        .clone();
    let done = h.wait(|s| s["plans"][0]["status"] == "proposed").await;
    let proposed = done["plans"][0].clone();
    assert_eq!(done["tasks"].as_array().unwrap().len(), 0);
    assert_eq!(done["runs"].as_array().unwrap().len(), 0);
    assert_eq!(proposed["source_inspected"], false);
    assert!(proposed["approval_path"].is_null());
    let mut proposal = proposed["proposals"][0].clone();
    let proposal_id = proposal
        .as_object_mut()
        .unwrap()
        .remove("proposal_id")
        .unwrap();
    proposal["title"] = json!("Human edited outcome");
    let selected = json!({"proposal_id":proposal_id,"proposal":proposal});
    let duplicate = json!({"expected_revision":proposed["revision"],"selected":[selected.clone(),selected.clone()]});
    h.call(
        "POST",
        &format!("/plans/{}/accept", plan["id"].as_str().unwrap()),
        duplicate,
        "duplicate-main-selection",
        400,
    )
    .await;
    let mut oversized = selected.clone();
    oversized["proposal"]["objective"] = json!("🦀".repeat(4000));
    oversized["proposal"]["acceptance_criteria"] =
        json!(["🦀".repeat(1000), "🦀".repeat(1000), "🦀".repeat(1000)]);
    h.call(
        "POST",
        &format!("/plans/{}/accept", plan["id"].as_str().unwrap()),
        json!({"expected_revision":proposed["revision"],"selected":[oversized]}),
        "oversize-main-selection",
        400,
    )
    .await;
    let acceptance = json!({"expected_revision":proposed["revision"],"selected":[selected]});
    h.call(
        "POST",
        "/execution/pause",
        json!({"paused":true}),
        "pause-main-review",
        200,
    )
    .await;
    let path = format!("/plans/{}/accept", plan["id"].as_str().unwrap());
    let accepted = h
        .call("POST", &path, acceptance.clone(), "accept-main-plan", 200)
        .await;
    let replay = h
        .call("POST", &path, acceptance.clone(), "accept-main-plan", 200)
        .await;
    assert_eq!(accepted, replay);
    let mut changed = acceptance.clone();
    changed["selected"][0]["proposal"]["title"] = json!("Changed payload");
    h.call("POST", &path, changed, "accept-main-plan", 409)
        .await;
    h.call("POST", &path, acceptance, "accept-main-again", 409)
        .await;
    let before = h.snapshot().await;
    assert_eq!(before["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(before["runs"].as_array().unwrap().len(), 0);
    let task = before["tasks"][0].clone();
    assert_eq!(task["title"], "Human edited outcome");
    assert_eq!(task["status"], "backlog");
    assert_eq!(task["owner"], "Coder");
    assert_eq!(task["automation_policy"], "manual");
    assert!(task["automation_revision"].is_null());
    assert_eq!(task["repository"], "fixture/a");
    assert_eq!(task["source_plan"]["plan_id"], plan["id"]);
    assert!(
        task["context"]
            .to_string()
            .contains("documented constraints")
    );
    let ready = h
        .call(
            "PATCH",
            &format!("/tasks/{}", task["id"].as_str().unwrap()),
            json!({"status":"ready","expected_revision":task["revision"]}),
            "ready-main-task",
            200,
        )
        .await["task"]
        .clone();
    assert_eq!(ready["automation_policy"], "ready_to_done");
    assert_eq!(ready["automation_revision"], ready["revision"]);
    assert_eq!(h.snapshot().await["runs"].as_array().unwrap().len(), 0);
    h.call(
        "POST",
        "/execution/pause",
        json!({"paused":false}),
        "resume-main-task",
        200,
    )
    .await;
    let finished = h.wait(|s| s["tasks"][0]["status"] == "done").await;
    assert_eq!(finished["runs"].as_array().unwrap().len(), 1);
    assert_eq!(
        h.audit().iter().filter(|a| a["action"] == "plan").count(),
        1
    );
    assert!(
        h.audit().iter().find(|a| a["action"] == "coder").unwrap()["task_context"]
            .to_string()
            .contains("documented constraints")
    );
    h.stop().await;
}

#[tokio::test]
async fn unknown_main_plan_blocks_generation_until_explicit_cleanup_without_replaying() {
    let mut h = Harness::new(json!({"planning_enabled":true,"bad_plan":true})).await;
    h.start().await;
    let input = json!({"goal":"Synthetic invalid model result","project":"Fixture","repository":"fixture/a"});
    let plan = h
        .call("POST", "/plans", input.clone(), "bad-main-plan", 202)
        .await["plan"]
        .clone();
    h.wait(|s| s["plans"][0]["status"] == "unknown").await;
    let calls = h.audit().len();
    h.call("POST", "/plans", input.clone(), "blocked-main-plan", 409)
        .await;
    assert_eq!(
        h.call("POST", "/plans", input.clone(), "bad-main-plan", 202)
            .await["plan"]["id"],
        plan["id"]
    );
    assert_eq!(h.audit().len(), calls);
    assert!(h.snapshot().await["tasks"].as_array().unwrap().is_empty());
    h.call(
        "POST",
        "/execution/pause",
        json!({"paused":true}),
        "pause-main-cleanup",
        200,
    )
    .await;
    h.call(
        "POST",
        &format!("/plans/{}/discard", plan["id"].as_str().unwrap()),
        json!({}),
        "discard-main-plan",
        200,
    )
    .await;
    h.wait(|s| s["plans"][0]["status"] == "discarded").await;
    assert_eq!(
        h.audit().iter().filter(|a| a["action"] == "plan").count(),
        1
    );
    assert_eq!(
        h.audit().iter().filter(|a| a["action"] == "cancel").count(),
        1
    );
    assert!(h.snapshot().await["runs"].as_array().unwrap().is_empty());
    h.stop().await;
}

#[tokio::test]
async fn main_input_bytes_and_proposal_authority_fail_before_task_creation() {
    let mut h = Harness::new(json!({"planning_enabled":true})).await;
    h.start().await;
    h.call("POST","/plans",json!({"goal":"🦀".repeat(6500),"context":"context","project":"Fixture","repository":"fixture/a"}),"oversize-main-goal",400).await;
    h.call(
        "POST",
        "/plans",
        json!({"goal":"Useful goal","project":"Fixture","repository":"fixture/a","status":"ready"}),
        "forged-main-goal",
        400,
    )
    .await;
    assert!(h.snapshot().await["plans"].as_array().unwrap().is_empty());
    assert!(h.audit().is_empty());
    h.stop().await;
}

#[tokio::test]
async fn interrupted_planning_restart_and_deadline_never_replay_generation() {
    for restart in [true, false] {
        let mut h=Harness::new(json!({"planning_enabled":true,"sleep_plan":3,"plan_stage_seconds":if restart{20}else{1}})).await;
        h.start().await;
        h.call("POST","/plans",json!({"goal":"Slow synthetic planner only","project":"Fixture","repository":"fixture/a"}),"interrupted-main-plan",202).await;
        h.wait(|_| h.audit().iter().any(|a| a["action"] == "plan"))
            .await;
        if restart {
            h.stop().await;
            let load = || {
                Factory::load(
                    &h.root.join("setup.json"),
                    KEY.as_bytes(),
                    &digest_file(&h.root.join("setup.json")).unwrap(),
                )
                .unwrap()
            };
            let placeholder =
                App::open_projects(&h.root.join("placeholder.sqlite"), KEY, 4311, load()).unwrap();
            let old = std::mem::replace(&mut h.app, placeholder);
            drop(old);
            h.app = App::open_projects(&h.root.join("board.sqlite"), KEY, 4311, load()).unwrap();
            h.engine = Some(h.app.start_execution());
            let response = h
                .request("POST", "/session", json!({"key":KEY}), None, false)
                .await;
            h.cookie = response.headers()["set-cookie"]
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .into();
            h.csrf = body(response).await["csrf_token"].as_str().unwrap().into();
        }
        h.wait(|s| s["plans"][0]["status"] == "unknown").await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            h.audit().iter().filter(|a| a["action"] == "plan").count(),
            1
        );
        assert!(h.snapshot().await["tasks"].as_array().unwrap().is_empty());
        assert!(h.snapshot().await["runs"].as_array().unwrap().is_empty());
        h.stop().await;
    }
}

#[tokio::test]
async fn queued_planning_rechecks_setup_freshness_but_cleanup_survives_drift() {
    let mut h = Harness::new(json!({"planning_enabled":true})).await;
    h.call(
        "POST",
        "/execution/pause",
        json!({"paused":false}),
        "resume-main-freshness",
        200,
    )
    .await;
    let plan = h
        .call(
            "POST",
            "/plans",
            json!({"goal":"A bounded goal","project":"Fixture","repository":"fixture/a"}),
            "queued-main-freshness",
            202,
        )
        .await["plan"]
        .clone();
    let path = h.root.join("setup.json");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.push(b'\n');
    std::fs::write(path, bytes).unwrap();
    h.engine = Some(h.app.start_execution());
    h.wait(|s| s["plans"][0]["status"] == "unknown").await;
    assert!(h.audit().is_empty());
    h.call(
        "POST",
        &format!("/plans/{}/discard", plan["id"].as_str().unwrap()),
        json!({}),
        "discard-main-freshness",
        200,
    )
    .await;
    h.wait(|s| s["plans"][0]["status"] == "discarded").await;
    assert_eq!(h.audit().len(), 1);
    assert_eq!(h.audit()[0]["action"], "cancel");
    h.stop().await;
}

#[tokio::test]
async fn dependent_urgent_task_waits_for_lower_priority_parent_done_then_wakes_without_run_poke() {
    let mut h = Harness::new(json!({})).await;
    let p = h.project("dependency-queue").await;
    let parent = h.task(&p, "Prerequisite", "ready", 4).await;
    let child=h.call("POST","/tasks",json!({"project_id":p["id"],"workflow":"engineering","title":"Urgent dependent","objective":"Run only after prerequisite Done","status":"ready","priority":1,"automation_policy":"ready_to_done","qa_policy":"skip","dependencies":[parent["id"]]}),"dependent",201).await["task"].clone();
    let refusal = h
        .call(
            "POST",
            &format!("/tasks/{}/run", child["id"].as_str().unwrap()),
            json!({"expected_revision":child["revision"]}),
            "blocked-run",
            409,
        )
        .await;
    assert_eq!(refusal["error"], "dependencies_unmet");
    assert!(h.snapshot().await["runs"].as_array().unwrap().is_empty());
    h.start_project(&p, "start-project").await;
    h.start().await;
    let done = h
        .wait(|s| {
            s["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|t| t["status"] == "done")
                .count()
                == 2
        })
        .await;
    let runs = done["runs"].as_array().unwrap();
    let pr = runs.iter().find(|r| r["task_id"] == parent["id"]).unwrap();
    let cr = runs.iter().find(|r| r["task_id"] == child["id"]).unwrap();
    assert!(pr["updated_at"].as_str().unwrap() <= cr["created_at"].as_str().unwrap());
    assert_eq!(cr["evidence"]["qa_status"], "skipped");
    h.stop().await;
}

#[tokio::test]
async fn main_typed_proposals_require_selected_prerequisites_and_review_controls_qa() {
    let mut h = Harness::new(json!({"planning_enabled":true,"plan_fields":true})).await;
    h.start().await;
    let job=h.call("POST","/plans",json!({"goal":"Explore then deliver","repository":"fixture/planning","project":"Planning"}),"goal-typed",202).await["plan"].clone();
    let proposed = h.wait(|s| s["plans"][0]["status"] == "proposed").await;
    let plan = &proposed["plans"][0];
    let proposals = plan["proposals"].as_array().unwrap();
    let select = |i: usize, qa: Option<&str>| {
        let mut p = proposals[i].clone();
        let id = p.as_object_mut().unwrap().remove("proposal_id").unwrap();
        let mut s = json!({"proposal_id":id,"proposal":p});
        if let Some(qa) = qa {
            s["qa_policy"] = json!(qa);
        }
        s
    };
    let path = format!("/plans/{}/accept", job["id"].as_str().unwrap());
    let orphan = h
        .call(
            "POST",
            &path,
            json!({"expected_revision":plan["revision"],"selected":[select(1,None)]}),
            "orphan-selection",
            400,
        )
        .await;
    assert_eq!(orphan["error"], "planning_dependency_unselected");
    let mut injected = select(0, None);
    injected["proposal"]["qa_policy"] = json!("skip");
    h.call(
        "POST",
        &path,
        json!({"expected_revision":plan["revision"],"selected":[injected]}),
        "ai-authority",
        400,
    )
    .await;
    let mut cyclic = select(0, None);
    cyclic["proposal"]["depends_on"] = json!([1]);
    h.call(
        "POST",
        &path,
        json!({"expected_revision":plan["revision"],"selected":[cyclic,select(1,None)]}),
        "cycle-selection",
        400,
    )
    .await;
    h.call(
        "POST",
        "/execution/pause",
        json!({"paused":true}),
        "pause-planning",
        200,
    )
    .await;
    let body = json!({"expected_revision":plan["revision"],"selected":[select(1,Some("required")),select(0,None)]});
    let accepted = h
        .call("POST", &path, body.clone(), "accept-typed", 200)
        .await;
    assert_eq!(
        h.call("POST", &path, body, "accept-typed", 200).await,
        accepted
    );
    let s = h.snapshot().await;
    assert!(s["runs"].as_array().unwrap().is_empty());
    let tasks = s["tasks"].as_array().unwrap();
    let research = tasks.iter().find(|t| t["task_type"] == "research").unwrap();
    let feature = tasks.iter().find(|t| t["task_type"] == "feature").unwrap();
    assert_eq!(research["qa_policy"], "skip");
    assert_eq!(research["labels"], json!(["discovery"]));
    assert_eq!(feature["qa_policy"], "required");
    assert_eq!(feature["dependencies"], json!([research["id"]]));
    assert_eq!(feature["status"], "backlog");
    assert_eq!(feature["automation_policy"], "manual");
    h.stop().await;
}

#[tokio::test]
async fn explicit_repository_run_requires_prepared_checks_for_its_current_revision() {
    let mut h = Harness::new(json!({})).await;
    let p = h.project("fresh-run").await;
    h.start_project(&p, "resume-fresh-project").await;
    let task=h.call("POST","/tasks",json!({"title":"Manual launch","objective":"Use fresh repository checks","repository":p["repository"],"workflow":"engineering","status":"ready","automation_policy":"manual"}),"fresh-run-task",201).await["task"].clone();
    h.call(
        "POST",
        "/execution/pause",
        json!({"paused":false}),
        "resume-before-run",
        200,
    )
    .await;
    let path = format!("/tasks/{}/run", task["id"].as_str().unwrap());
    let request = json!({"expected_revision":task["revision"]});
    let refusal = h
        .call("POST", &path, request.clone(), "explicit-fresh-run", 409)
        .await;
    assert_eq!(refusal["error"], "project_needs_preparation");
    assert!(h.snapshot().await["runs"].as_array().unwrap().is_empty());
    assert!(h.audit().is_empty());
    h.engine = Some(h.app.start_execution());
    let expected = format!("{}:{}", task["id"].as_str().unwrap(), task["revision"]);
    h.wait(|s| {
        s["projects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["runtime_status"] == "configured" && p["checked_task_revision"] == expected)
    })
    .await;
    let admitted = h
        .call("POST", &path, request.clone(), "explicit-fresh-run", 202)
        .await;
    h.wait(|s| {
        s["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == task["id"] && t["status"] == "done")
    })
    .await;
    assert_eq!(
        h.call("POST", &path, request, "explicit-fresh-run", 202)
            .await,
        admitted
    );
    h.stop().await;
}

#[tokio::test]
async fn no_install_unit_ticket_discovers_while_paused_without_cache_or_worker_and_rejects_unsafe_source()
 {
    let mut h = Harness::new(json!({})).await;
    let source = h.repo("a");
    write(&source.join("package.json"),json!({"type":"module","scripts":{"test":"node --test"},"dependencies":{"express":"^5.1.0","dotenv":"^16.6.1"}}).to_string(),0o600);
    write(
        &source.join("test/logic.test.mjs"),
        "import test from 'node:test';test('synthetic',()=>{});\n",
        0o600,
    );
    git(&source, &["add", "."]);
    git(
        &source,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            "synthetic no-install unit source",
        ],
    );
    h.call("POST","/tasks",json!({"repository":"fixture/a","title":"Unit profile","objective":"No application startup claim","acceptance_criteria":["units only"],"status":"ready"}),"unit-ticket",201).await;
    h.engine = Some(h.app.start_execution());
    let s = h
        .wait(|s| s["projects"][0]["runtime_status"] == "configured")
        .await;
    assert_eq!(s["projects"][0]["profile"]["id"], "node-unit-syntax-v1");
    assert_eq!(
        s["projects"][0]["validation_summary"],
        "Node unit tests · JavaScript syntax (no install or app build)"
    );
    assert!(s["runs"].as_array().unwrap().is_empty());
    assert!(h.audit().is_empty());
    // A source symlink must be rejected at discovery before any model budget.
    std::os::unix::fs::symlink("/private/tmp", source.join("test/unsafe")).unwrap();
    git(&source, &["add", "."]);
    git(
        &source,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            "synthetic unsafe source",
        ],
    );
    let p = &s["projects"][0];
    h.call(
        "PATCH",
        &format!("/projects/{}", p["id"].as_str().unwrap()),
        json!({"expected_revision":p["revision"],"reconnect":true}),
        "unit-recheck",
        200,
    )
    .await;
    let s = h
        .wait(|s| s["projects"][0]["runtime_status"] == "failed")
        .await;
    assert_eq!(s["projects"][0]["paused"], true);
    assert!(s["runs"].as_array().unwrap().is_empty());
    assert!(h.audit().is_empty());
    h.stop().await;
}

#[tokio::test]
async fn native_public_repository_bypasses_finite_profiles_and_pins_ticket_agent() {
    let mut h = Harness::new(json!({"native":true})).await;
    h.engine = Some(h.app.start_execution());
    let snapshot = h.snapshot().await;
    assert_eq!(snapshot["coding_agents"]["default_coder"], "coder");
    assert_eq!(
        snapshot["coding_agents"]["agents"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let manual=h.call("POST","/tasks",json!({"workflow":"manual","title":"Manual tracking","objective":"No agent needed","coding_agent_id":null}),"native-manual",201).await["task"].clone();
    assert!(manual["coding_agent_id"].is_null());
    let task=h.call("POST","/tasks",json!({"project":"Public arbitrary stack","repository":"fixture/public","title":"Native generic setup","objective":"No finite package profile required","coding_agent_id":"specialist","status":"ready"}),"native-task",201).await["task"].clone();
    assert_eq!(task["coding_agent_id"], "specialist");
    assert_eq!(
        h.call(
            "PATCH",
            &format!("/tasks/{}", task["id"].as_str().unwrap()),
            json!({"expected_revision":task["revision"],"coding_agent_id":"qa"}),
            "qa-not-coder",
            409,
        )
        .await["error"],
        "agent_unavailable"
    );
    assert_eq!(
        h.call(
            "PATCH",
            &format!("/tasks/{}", task["id"].as_str().unwrap()),
            json!({"expected_revision":task["revision"],"coding_agent_id":"missing"}),
            "unknown-coder",
            409,
        )
        .await["error"],
        "agent_unavailable"
    );
    h.call(
        "PATCH",
        &format!("/tasks/{}", task["id"].as_str().unwrap()),
        json!({"expected_revision":task["revision"],"coding_agent_id":42}),
        "malformed-coder",
        400,
    )
    .await;
    let ready = h
        .wait(|s| {
            s["projects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["runtime_status"] == "configured" || p["runtime_status"] == "failed")
        })
        .await;
    assert!(ready["runs"].as_array().unwrap().is_empty());
    let project = &ready["projects"][0];
    assert_eq!(project["runtime_status"], "configured", "{ready}");
    let approval_path =
        std::fs::read_dir(h.root.join("actions").join(project["id"].as_str().unwrap()))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("execution-")
            })
            .unwrap();
    let approval: Value = serde_json::from_slice(&std::fs::read(approval_path).unwrap()).unwrap();
    assert!(approval["payload"]["adapter_policy"]["validation_profile"].is_null());
    assert_eq!(
        approval["payload"]["adapter_policy"]["native_agent"]["default_coder"],
        "coder"
    );
    // Public projection never leaks native program/home/catalog paths.
    assert!(!project.to_string().contains("catalog.json"));
    assert!(h.audit().is_empty());
    h.stop().await;
}

#[tokio::test]
async fn system_agent_settings_freeze_runs_and_plans_while_new_work_uses_new_defaults() {
    let mut h = Harness::new(json!({"planning_enabled":true,"sleep_coder":0.35})).await;
    let project = h.project("a").await;
    let first = h.task(&project, "frozen-agent-first", "ready", 3).await;
    let initial = h.snapshot().await["system"].clone();
    let mut update = json!({"expected_revision":initial["revision"],"agents":initial["agents"],"defaults":initial["defaults"]});
    for agent in update["agents"].as_array_mut().unwrap() {
        agent["model"] = json!("old-reviewed-model");
        agent["instructions"] = json!(format!(
            "Original {} instructions",
            agent["role"].as_str().unwrap()
        ));
    }
    h.call("PUT", "/system/agents", update, "configure-old-agents", 200)
        .await;
    h.start_project(&project, "start-system-project").await;
    h.start().await;
    let running = h
        .wait(|s| {
            s["runs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["stage"] == "coder")
        })
        .await;
    let run = running["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["task_id"] == first["id"])
        .unwrap()
        .clone();
    assert_eq!(run["agents"]["revision"], 2);
    assert_eq!(run["agents"]["qa"]["model"], "old-reviewed-model");
    let mut update = json!({"expected_revision":running["system"]["revision"],"agents":running["system"]["agents"],"defaults":running["system"]["defaults"]});
    for agent in update["agents"].as_array_mut().unwrap() {
        agent["model"] = json!("new-reviewed-model");
        agent["instructions"] = json!(format!(
            "New {} instructions",
            agent["role"].as_str().unwrap()
        ));
    }
    let qa = json!({"id":"new-qa","label":"New QA","role":"qa","instructions":"Independent new QA instructions","model":"new-qa-model","enabled":true});
    update["agents"]
        .as_array_mut()
        .unwrap()
        .retain(|a| a["id"] != "qa");
    update["agents"].as_array_mut().unwrap().push(qa);
    update["defaults"]["qa"] = json!("new-qa");
    h.call("PUT", "/system/agents", update, "configure-new-agents", 200)
        .await;
    let done = h
        .wait(|s| {
            s["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["id"] == first["id"] && t["status"] == "done")
        })
        .await;
    assert_eq!(done["runs"][0]["agents"]["qa"]["id"], "qa");
    for event in h.audit().iter().filter(|e| e["run_id"] == run["id"]) {
        assert_eq!(event["agent_config"]["revision"], 2);
        assert_eq!(event["agent_config"]["qa"]["model"], "old-reviewed-model");
    }
    let second = h.task(&project, "new-agent-second", "ready", 3).await;
    let done = h
        .wait(|s| {
            s["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["id"] == second["id"] && t["status"] == "done")
        })
        .await;
    let run = done["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["task_id"] == second["id"])
        .unwrap();
    assert_eq!(run["agents"]["revision"], 3);
    assert_eq!(run["agents"]["qa"]["id"], "new-qa");
    // Main is frozen when generation is admitted, including a queued plan before its process starts.
    h.stop().await;
    // Use a separate supported app after releasing old state; no model process is launched here.
    drop(h);
    let h = Harness::new(json!({"planning_enabled":true})).await;
    h.call(
        "POST",
        "/execution/pause",
        json!({"paused":false}),
        "resume-main-settings",
        200,
    )
    .await;
    let plan=h.call("POST","/plans",json!({"goal":"Propose bounded work","context":"No provider fixture","repository":"fixture/a","project":"A"}),"plan-old-settings",202).await["plan"].clone();
    let initial = h.snapshot().await["system"].clone();
    let mut update = json!({"expected_revision":initial["revision"],"agents":initial["agents"],"defaults":initial["defaults"]});
    update["agents"][0]["model"] = json!("new-main-model");
    update["agents"][0]["instructions"] = json!("New Main instructions");
    h.call(
        "PUT",
        "/system/agents",
        update,
        "update-main-after-admission",
        200,
    )
    .await;
    let current = h.snapshot().await;
    assert_eq!(current["plans"][0]["agent_revision"], 1);
    assert_eq!(current["plans"][0]["agent"]["model"], "");
    assert_eq!(current["system"]["agents"][0]["model"], "new-main-model");
    assert_eq!(plan["agent"]["id"], "main");
    assert!(h.audit().is_empty());
}

#[tokio::test]
async fn disabled_agents_block_fresh_work_without_skipping_qa_or_breaking_replay() {
    let mut h = Harness::new(json!({"planning_enabled":true})).await;
    let project = h.project("a").await;
    let task = h.task(&project, "disabled-qa", "ready", 3).await;
    let initial = h.snapshot().await["system"].clone();
    let mut update = json!({"expected_revision":initial["revision"],"agents":initial["agents"],"defaults":initial["defaults"]});
    update["agents"][2]["enabled"] = json!(false);
    h.call("PUT", "/system/agents", update, "disable-default-qa", 200)
        .await;
    h.start_project(&project, "start-disabled-project").await;
    h.start().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(h.snapshot().await["runs"].as_array().unwrap().is_empty());
    assert!(h.audit().is_empty());
    assert_eq!(
        h.call(
            "POST",
            &format!("/tasks/{}/run", task["id"].as_str().unwrap()),
            json!({"expected_revision":task["revision"]}),
            "run-disabled-qa",
            409
        )
        .await["error"],
        "agent_unavailable"
    );
    let current = h.snapshot().await["system"].clone();
    let mut update = json!({"expected_revision":current["revision"],"agents":current["agents"],"defaults":current["defaults"]});
    update["agents"][2]["enabled"] = json!(true);
    update["agents"][0]["enabled"] = json!(false);
    h.call(
        "PUT",
        "/system/agents",
        update,
        "enable-qa-disable-main",
        200,
    )
    .await;
    assert_eq!(
        h.call(
            "POST",
            "/plans",
            json!({"goal":"Bounded goal","repository":"fixture/a","project":"A"}),
            "disabled-main-plan",
            409
        )
        .await["error"],
        "agent_unavailable"
    );
    h.wait(|s| {
        s["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == task["id"] && t["status"] == "done")
    })
    .await;
    h.stop().await;
}

#[tokio::test]
async fn historical_task_administration_survives_removed_coding_agent() {
    let mut h = Harness::new(json!({})).await;
    let project = h.project("a").await;
    let task = h.task(&project, "historic-agent", "ready", 3).await;
    h.start_project(&project, "historic-start").await;
    h.start().await;
    let done = h
        .wait(|s| {
            s["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["id"] == task["id"] && t["status"] == "done")
        })
        .await;
    let registry = &done["system"];
    let mut update = json!({"expected_revision":registry["revision"],"agents":registry["agents"],"defaults":registry["defaults"]});
    update["agents"]
        .as_array_mut()
        .unwrap()
        .retain(|a| a["id"] != "coder");
    update["agents"].as_array_mut().unwrap().push(json!({"id":"replacement-coder","label":"Replacement","role":"coder","instructions":"Implement","model":"","enabled":true}));
    update["defaults"]["coder"] = json!("replacement-coder");
    h.call(
        "PUT",
        "/system/agents",
        update,
        "remove-historical-coder",
        200,
    )
    .await;
    let task = done["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == task["id"])
        .unwrap();
    let path = format!("/tasks/{}", task["id"].as_str().unwrap());
    let archived = h
        .call(
            "POST",
            &format!("{path}/archive"),
            json!({"expected_revision":task["revision"]}),
            "archive-after-coder-delete",
            200,
        )
        .await["task"]
        .clone();
    assert_eq!(archived["coding_agent_id"], "coder");
    let restored = h
        .call(
            "POST",
            &format!("{path}/restore"),
            json!({"expected_revision":archived["revision"]}),
            "restore-after-coder-delete",
            200,
        )
        .await["task"]
        .clone();
    assert_eq!(restored["coding_agent_id"], "coder");
    assert_eq!(h.snapshot().await["runs"].as_array().unwrap().len(), 1);
    h.stop().await;
}

#[tokio::test]
async fn near_limit_registry_content_forms_real_native_run_and_plan_approvals() {
    let mut h = Harness::new(json!({"native":true,"planning_enabled":true})).await;
    let initial = h.snapshot().await["system"].clone();
    let mut update = json!({"expected_revision":initial["revision"],"agents":initial["agents"],"defaults":initial["defaults"]});
    for agent in update["agents"].as_array_mut().unwrap() {
        if ["main", "coder", "qa"].contains(&agent["id"].as_str().unwrap()) {
            agent["instructions"] = json!("x\"".repeat(800));
        }
    }
    assert!(update.to_string().len() > 7000 && update.to_string().len() < 8192);
    h.call(
        "PUT",
        "/system/agents",
        update.clone(),
        "near-limit-save",
        200,
    )
    .await;
    let task=h.call("POST","/tasks",json!({"repository":"fixture/public","title":"Native size bound","objective":"Synthetic only","status":"ready","automation_policy":"manual"}),"near-limit-task",201).await["task"].clone();
    h.engine = Some(h.app.start_execution());
    h.wait(|s| {
        s["projects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["runtime_status"] == "configured")
    })
    .await;
    h.call(
        "POST",
        "/execution/pause",
        json!({"paused":false}),
        "near-limit-resume",
        200,
    )
    .await;
    let run = h
        .call(
            "POST",
            &format!("/tasks/{}/run", task["id"].as_str().unwrap()),
            json!({"expected_revision":task["revision"]}),
            "near-limit-run",
            202,
        )
        .await["run"]
        .clone();
    assert_eq!(run["agents"]["revision"], 2);
    h.wait(|s| {
        s["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == task["id"] && t["status"] == "done")
    })
    .await;
    let plan = h
        .call(
            "POST",
            "/plans",
            json!({"goal":"Bounded sizing plan","project":"Public","repository":"fixture/public"}),
            "near-limit-plan",
            202,
        )
        .await["plan"]
        .clone();
    assert_eq!(plan["agent_revision"], 2);
    h.wait(|s| {
        s["plans"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == plan["id"] && p["status"] == "proposed")
    })
    .await;
    let current = h.snapshot().await["system"].clone();
    let mut disable = json!({"expected_revision":current["revision"],"agents":current["agents"],"defaults":current["defaults"]});
    for agent in disable["agents"].as_array_mut().unwrap() {
        agent["enabled"] = json!(false);
    }
    h.call(
        "PUT",
        "/system/agents",
        disable,
        "disable-after-admission",
        200,
    )
    .await;
    let replay = h
        .call(
            "POST",
            &format!("/tasks/{}/run", task["id"].as_str().unwrap()),
            json!({"expected_revision":task["revision"]}),
            "near-limit-run",
            202,
        )
        .await;
    assert_eq!(replay["run"]["id"], run["id"]);
    assert_eq!(h.snapshot().await["runs"].as_array().unwrap().len(), 1);
    update["expected_revision"] = json!(3);
    for agent in update["agents"].as_array_mut().unwrap() {
        agent["instructions"] = json!("\"".repeat(2000));
    }
    assert_eq!(
        h.call("PUT", "/system/agents", update, "oversize-registry", 400)
            .await["error"],
        "agent_config_too_large"
    );
    assert_eq!(h.snapshot().await["system"]["revision"], 3);
    h.stop().await;
}

#[tokio::test]
async fn restarted_run_cleanup_uses_original_agents_after_registry_changes() {
    let mut h = Harness::new(json!({"sleep_coder":2})).await;
    let project = h.project("a").await;
    h.task(&project, "restart-frozen-actors", "ready", 3).await;
    h.start_project(&project, "restart-actors-start").await;
    h.start().await;
    let active = h
        .wait(|_| h.audit().iter().any(|a| a["action"] == "coder"))
        .await;
    let run = active["runs"][0].clone();
    let current = &active["system"];
    let mut update = json!({"expected_revision":current["revision"],"agents":current["agents"],"defaults":current["defaults"]});
    for agent in update["agents"].as_array_mut().unwrap() {
        agent["model"] = json!("new-model-after-admission");
        agent["enabled"] = json!(false);
    }
    h.call(
        "PUT",
        "/system/agents",
        update,
        "change-agents-before-restart",
        200,
    )
    .await;
    h.stop().await;
    let factory = Factory::load(
        &h.root.join("setup.json"),
        KEY.as_bytes(),
        &digest_file(&h.root.join("setup.json")).unwrap(),
    )
    .unwrap();
    let dummy = App::open(&h.root.join("unused-restart-agents.sqlite"), KEY, 4311).unwrap();
    drop(std::mem::replace(&mut h.app, dummy));
    h.app = App::open_projects(&h.root.join("board.sqlite"), KEY, 4311, factory).unwrap();
    h.engine = Some(h.app.start_execution());
    let restarted = h.snapshot().await;
    assert_eq!(restarted["system"]["revision"], 2);
    assert_eq!(restarted["runs"][0]["status"], "unknown");
    assert_eq!(restarted["runs"][0]["agents"]["revision"], 1);
    h.call(
        "POST",
        &format!("/runs/{}/cancel", run["id"].as_str().unwrap()),
        json!({}),
        "cleanup-frozen-agents",
        202,
    )
    .await;
    h.wait(|s| s["runs"][0]["status"] == "cancelled").await;
    let audit = h.audit();
    assert_eq!(audit.iter().filter(|a| a["action"] == "coder").count(), 1);
    for action in audit
        .iter()
        .filter(|a| a["action"] == "cancel" || a["action"] == "observe")
    {
        assert_eq!(action["agent_config"]["revision"], 1);
        assert_eq!(action["agent_config"]["qa"]["model"], "");
    }
    h.stop().await;
}

#[tokio::test]
async fn card_priority_edit_preserves_current_queue_enrollment_and_ready_time() {
    let h = Harness::new(json!({})).await;
    let project = h.project("priority-edit").await;
    let task = h.task(&project, "queued-priority-edit", "ready", 3).await;
    let path = format!("/tasks/{}", task["id"].as_str().unwrap());
    let edited = h.call("PATCH", &path, json!({"expected_revision":task["revision"],"priority":1,"automation_policy":"ready_to_done"}), "card-priority-edit", 200).await["task"].clone();
    assert_eq!(edited["priority"], 1);
    assert_eq!(edited["ready_at"], task["ready_at"]);
    assert_eq!(edited["automation_policy"], "ready_to_done");
    assert_eq!(edited["automation_revision"], edited["revision"]);
    for key in [
        "title",
        "objective",
        "qa_policy",
        "merge_policy",
        "status",
        "repository",
    ] {
        assert_eq!(edited[key], task[key]);
    }
    h.call("PATCH", &path, json!({"expected_revision":task["revision"],"priority":4,"automation_policy":"ready_to_done"}), "stale-card-priority", 409).await;
    let snapshot = h.snapshot().await;
    assert!(snapshot["runs"].as_array().unwrap().is_empty());
    let current = snapshot["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == task["id"])
        .unwrap();
    assert_eq!(current["priority"], 1);
    assert_eq!(current["revision"], edited["revision"]);
    let manual = h.call("POST", "/tasks", json!({"project_id":project["id"],"title":"Explicit Run ticket","objective":"Manual processing remains manual","workflow":"engineering","status":"ready","automation_policy":"manual"}), "manual-card-task", 201).await["task"].clone();
    let manual = h.call("PATCH", &format!("/tasks/{}", manual["id"].as_str().unwrap()), json!({"expected_revision":manual["revision"],"priority":1,"automation_policy":"manual"}), "manual-card-priority", 200).await["task"].clone();
    assert_eq!(manual["automation_policy"], "manual");
    assert!(manual["automation_revision"].is_null());
}

#[tokio::test]
async fn kanban_ready_drop_enrolls_manual_processing_and_backlog_removes_it() {
    let mut h = Harness::new(json!({})).await;
    let project = h.project("drag-ready").await;
    let task=h.call("POST","/tasks",json!({"project_id":project["id"],"title":"drag-ready-ticket","objective":"Synthetic drag command validation","workflow":"engineering","status":"backlog","automation_policy":"manual","priority":2,"qa_policy":"required","merge_policy":"auto"}),"drag-task",201).await["task"].clone();
    let path = format!("/tasks/{}", task["id"].as_str().unwrap());
    let ready = h
        .call(
            "PATCH",
            &path,
            json!({"expected_revision":1,"status":"ready","automation_policy":"ready_to_done"}),
            "drag-first-ready",
            200,
        )
        .await["task"]
        .clone();
    assert_eq!(ready["automation_revision"], ready["revision"]);
    let backlog=h.call("PATCH",&path,json!({"expected_revision":ready["revision"],"status":"backlog","automation_policy":"manual"}),"drag-backlog",200).await["task"].clone();
    assert_eq!(backlog["status"], "backlog");
    assert!(backlog["automation_revision"].is_null());
    assert!(backlog["ready_at"].is_null());
    h.call("PATCH",&path,json!({"expected_revision":ready["revision"],"status":"ready","automation_policy":"ready_to_done"}),"drag-stale",409).await;
    let ready=h.call("PATCH",&path,json!({"expected_revision":backlog["revision"],"status":"ready","automation_policy":"ready_to_done"}),"drag-final-ready",200).await["task"].clone();
    for key in ["priority", "qa_policy", "merge_policy"] {
        assert_eq!(ready[key], task[key]);
    }
    h.start_project(&project, "drag-start-project").await;
    h.start().await;
    let s = h
        .wait(|s| {
            s["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["id"] == task["id"] && t["status"] == "done")
        })
        .await;
    let runs = s["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["task_revision"], ready["revision"]);
    assert_eq!(runs[0]["status"], "succeeded");
    assert_eq!(runs[0]["evidence"]["qa_status"], "passed");
    h.stop().await;
}

async fn research_task(h: &Harness, name: &str, qa: &str, priority: u32) -> Value {
    h.call("POST","/tasks",json!({"title":name,"objective":"Research a bounded synthetic question and report sources.","acceptance_criteria":["Explain conclusions and limitations with sources."],"workflow":"engineering","task_type":"research","qa_policy":qa,"status":"ready","automation_policy":"ready_to_done","priority":priority}),&uuid::Uuid::new_v4().to_string(),201).await["task"].clone()
}
#[tokio::test]
async fn repository_free_research_reports_done_downloads_and_never_publishes() {
    let mut h = Harness::new(json!({"native":true})).await;
    let task = research_task(&h, "Research-only", "skip", 3).await;
    h.call(
        "PATCH",
        &format!("/tasks/{}", task["id"].as_str().unwrap()),
        json!({"expected_revision":1,"status":"done"}),
        "no-fake-done",
        503,
    )
    .await;
    h.start().await;
    let s = h
        .wait(|s| {
            s["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["id"] == task["id"] && t["status"] == "done")
        })
        .await;
    assert!(s["projects"].as_array().unwrap().is_empty());
    let run = &s["runs"][0];
    assert_eq!(run["kind"], "research");
    assert_eq!(run["evidence"]["qa_status"], "skipped");
    assert!(run["head_sha"].is_null());
    assert!(run["pull_request_url"].is_null());
    assert_eq!(
        h.audit()
            .iter()
            .map(|a| a["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["research"]
    );
    let path = format!("/runs/{}/report.md", run["id"].as_str().unwrap());
    let response = h.request("GET", &path, Value::Null, None, true).await;
    assert_eq!(response.status(), 200);
    assert!(
        response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("attachment")
    );
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&bytes).contains("Synthetic research report"));
    assert_eq!(
        h.request("GET", &path, Value::Null, None, false)
            .await
            .status(),
        401
    );
    let source = h
        .request(
            "GET",
            &format!("/runs/{}/sources.json", run["id"].as_str().unwrap()),
            Value::Null,
            None,
            true,
        )
        .await;
    assert_eq!(body(source).await[0]["url"], "https://example.org/research");
    // A valid-shaped report edited outside its verified receipt must never be downloaded.
    rusqlite::Connection::open(h.root.join("board.sqlite"))
        .unwrap()
        .execute(
            "UPDATE runs SET document=json_set(document, '$.report.markdown', ?1) WHERE id=?2",
            rusqlite::params![
                "A different substantive report with no matching review digest.",
                run["id"].as_str().unwrap()
            ],
        )
        .unwrap();
    assert_eq!(
        h.request("GET", &path, Value::Null, None, true)
            .await
            .status(),
        400
    );
    h.stop().await;
}
#[tokio::test]
async fn repository_free_research_required_qa_repairs_exact_report_or_exhausts() {
    for rejects in [1, 3] {
        let mut h = Harness::new(json!({"native":true,"research_rejections":rejects})).await;
        research_task(&h, &format!("Reviewed report{rejects}"), "required", 3).await;
        h.start().await;
        let s = h
            .wait(|s| {
                s["runs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| ["succeeded", "failed"].contains(&r["status"].as_str().unwrap_or("")))
            })
            .await;
        let r = &s["runs"][0];
        assert_eq!(
            r["status"],
            if rejects == 1 { "succeeded" } else { "failed" }
        );
        assert_eq!(r["repair_count"], if rejects == 1 { 1 } else { 2 });
        assert_eq!(
            r["review_history"].as_array().unwrap().len(),
            if rejects == 1 { 2 } else { 3 }
        );
        assert!(
            r["review_history"]
                .as_array()
                .unwrap()
                .iter()
                .all(|v| v["report_digest"] == r["report_digest"])
        );
        assert!(
            h.audit()
                .iter()
                .all(|a| ["research", "research_qa"].contains(&a["action"].as_str().unwrap()))
        );
        h.stop().await;
    }
}
#[tokio::test]
async fn repository_free_research_invalid_result_reconciles_only_through_worker_cleanup() {
    let mut h = Harness::new(json!({"native":true,"research_wrong_digest":true})).await;
    research_task(&h, "Wrong digest", "required", 3).await;
    h.start().await;
    let s = h.wait(|s| s["runs"][0]["status"] == "unknown").await;
    let id = s["runs"][0]["id"].as_str().unwrap();
    h.call(
        "POST",
        "/execution/pause",
        json!({"paused":true}),
        "pause-research",
        200,
    )
    .await;
    h.call(
        "POST",
        &format!("/runs/{id}/reconcile"),
        json!({}),
        "reconcile-research",
        202,
    )
    .await;
    h.wait(|s| s["runs"][0]["status"] == "cancelled").await;
    assert_eq!(
        h.audit()
            .iter()
            .map(|a| a["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["research", "research_qa", "cancel"]
    );
    h.stop().await;
}
#[tokio::test]
async fn repository_free_research_priority_fifo_dependencies_and_input_guards() {
    let mut h = Harness::new(json!({"native":true})).await;
    for value in [json!(42), json!([]), json!({})] {
        h.call("POST","/tasks",json!({"title":"Bad repository","objective":"Reject invalid parameter","workflow":"engineering","task_type":"research","repository":value}),&uuid::Uuid::new_v4().to_string(),400).await;
    }
    h.call("POST","/tasks",json!({"title":"No repository feature","objective":"No fallback","workflow":"engineering","task_type":"feature"}),"feature-repo-required",400).await;
    let no_priority = research_task(&h, "No priority report", "skip", 0).await;
    let parent = research_task(&h, "Parent report", "skip", 3).await;
    let child=h.call("POST","/tasks",json!({"title":"Urgent dependent","objective":"Wait for report parent","workflow":"engineering","task_type":"research","status":"ready","automation_policy":"ready_to_done","priority":1,"dependencies":[parent["id"]]}),"dependent-report",201).await["task"].clone();
    h.start().await;
    let s = h
        .wait(|s| {
            s["runs"].as_array().unwrap().len() == 3
                && s["runs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|r| r["status"] == "succeeded")
        })
        .await;
    let order: Vec<_> = s["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["task_id"].clone())
        .collect();
    assert_eq!(
        order,
        vec![
            parent["id"].clone(),
            child["id"].clone(),
            no_priority["id"].clone()
        ]
    );
    h.stop().await;
}

#[tokio::test]
async fn repository_free_main_accepts_only_research_then_ready_reports_done() {
    let mut h = Harness::new(json!({"native":true,"planning_enabled":true})).await;
    h.start().await;
    h.call(
        "POST",
        "/plans",
        json!({"goal":"Research a synthetic question","project":"Research"}),
        "blank-repository-goal",
        202,
    )
    .await;
    let s = h.wait(|s| s["plans"][0]["status"] == "proposed").await;
    let plan = &s["plans"][0];
    let path = format!("/plans/{}/accept", plan["id"].as_str().unwrap());
    let mut proposal = plan["proposals"][0].clone();
    let pid = proposal
        .as_object_mut()
        .unwrap()
        .remove("proposal_id")
        .unwrap();
    let mut bad = proposal.clone();
    bad["task_type"] = json!("feature");
    h.call("POST",&path,json!({"expected_revision":plan["revision"],"selected":[{"proposal_id":pid,"proposal":bad}]}),"reject-engineering-without-repo",400).await;
    h.call(
        "POST",
        "/execution/pause",
        json!({"paused":true}),
        "pause-for-accept",
        200,
    )
    .await;
    let accepted=h.call("POST",&path,json!({"expected_revision":plan["revision"],"selected":[{"proposal_id":pid,"proposal":proposal}]}),"accept-research-proposal",200).await;
    assert_eq!(h.snapshot().await["runs"].as_array().unwrap().len(), 0);
    let tid = accepted["plan"]["accepted_task_ids"][0].as_str().unwrap();
    let s = h.snapshot().await;
    let task = s["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == tid)
        .unwrap();
    assert_eq!(task["status"], "backlog");
    assert_eq!(task["qa_policy"], "skip");
    assert!(task["repository"].is_null());
    h.call(
        "PATCH",
        &format!("/tasks/{tid}"),
        json!({"expected_revision":task["revision"],"status":"ready"}),
        "ready-research-plan",
        200,
    )
    .await;
    h.call(
        "POST",
        "/execution/pause",
        json!({"paused":false}),
        "resume-research-plan",
        200,
    )
    .await;
    h.wait(|s| {
        s["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == tid && t["status"] == "done")
    })
    .await;
    assert_eq!(
        h.audit()
            .iter()
            .map(|a| a["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["plan", "research"]
    );
    h.stop().await;
}
#[tokio::test]
async fn repository_free_research_restart_unknown_does_not_replay_or_block_repository_queue() {
    let mut h = Harness::new(json!({"native":true,"research_wrong_digest":true})).await;
    h.repo("b");
    let engineering=h.call("POST","/tasks",json!({"title":"Engineering in parallel","objective":"Synthetic independent repository","workflow":"engineering","repository":"fixture/b","status":"ready","automation_policy":"ready_to_done"}),"parallel-engineering",201).await["task"].clone();
    research_task(&h, "Uncertain report", "required", 1).await;
    h.start().await;
    let s = h
        .wait(|s| {
            s["runs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["kind"] == "research" && r["status"] == "unknown")
                && s["tasks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|t| t["id"] == engineering["id"] && t["status"] == "done")
        })
        .await;
    let rid = s["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "research")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    h.stop().await;
    let load = || {
        Factory::load(
            &h.root.join("setup.json"),
            KEY.as_bytes(),
            &digest_file(&h.root.join("setup.json")).unwrap(),
        )
        .unwrap()
    };
    let placeholder =
        App::open_projects(&h.root.join("placeholder.sqlite"), KEY, 4311, load()).unwrap();
    drop(std::mem::replace(&mut h.app, placeholder));
    h.app = App::open_projects(&h.root.join("board.sqlite"), KEY, 4311, load()).unwrap();
    h.engine = Some(h.app.start_execution());
    let response = h
        .request("POST", "/session", json!({"key":KEY}), None, false)
        .await;
    h.cookie = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .into();
    h.csrf = body(response).await["csrf_token"].as_str().unwrap().into();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let research = h
        .audit()
        .iter()
        .filter(|a| a["action"] == "research")
        .count();
    assert_eq!(research, 1);
    h.call(
        "POST",
        &format!("/runs/{rid}/cancel"),
        json!({}),
        "stop-restored-research",
        202,
    )
    .await;
    h.wait(|s| {
        s["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == rid && r["status"] == "cancelled")
    })
    .await;
    assert_eq!(h.audit().last().unwrap()["action"], "cancel");
    h.stop().await;
}
