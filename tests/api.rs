use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use workboard_rs::App;

const API: &str = "/api/workboard/v1";
const KEY: &str = "0123456789abcdef0123456789abcdef";
struct Harness {
    app: App,
    root: tempfile::TempDir,
    cookie: String,
    csrf: String,
}
impl Harness {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let app = App::open(&root.path().join("board.sqlite"), KEY, 4311).unwrap();
        let mut h = Self {
            app,
            root,
            cookie: String::new(),
            csrf: String::new(),
        };
        h.login().await;
        h
    }
    async fn login(&mut self) {
        let response = self
            .send("POST", "/session", Some(json!({"key":KEY})), None, false)
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        self.cookie = response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        self.csrf = body(response).await["csrf_token"]
            .as_str()
            .unwrap()
            .to_string();
    }
    async fn send(
        &self,
        method: &str,
        path: &str,
        value: Option<Value>,
        key: Option<&str>,
        signed: bool,
    ) -> Response {
        let mut req = Request::builder()
            .method(method)
            .uri(format!("{API}{path}"))
            .header("host", "127.0.0.1:4311");
        if method != "GET" {
            req = req
                .header("origin", "http://127.0.0.1:4311")
                .header("content-type", "application/json");
        }
        if signed {
            req = req
                .header("cookie", &self.cookie)
                .header("x-csrf-token", &self.csrf);
        }
        if let Some(key) = key {
            req = req.header("idempotency-key", key);
        }
        self.app
            .router()
            .oneshot(
                req.body(Body::from(value.map(|v| v.to_string()).unwrap_or_default()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }
    async fn command(
        &self,
        path: &str,
        method: &str,
        value: Value,
        key: &str,
        status: u16,
    ) -> Value {
        let response = self.send(method, path, Some(value), Some(key), true).await;
        assert_eq!(response.status().as_u16(), status);
        body(response).await
    }
    async fn create(&self, workflow: &str) -> Value {
        let mut input =
            json!({"title":"Build small","objective":"A concrete outcome","workflow":workflow});
        if workflow == "engineering" {
            input["repository"] = json!("owner/repository");
        }
        self.command("/tasks", "POST", input, "create-task", 201)
            .await["task"]
            .clone()
    }
}
async fn body(response: Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[tokio::test]
async fn authentication_origin_csrf_and_logout_are_enforced() {
    let h = Harness::new().await;
    assert_eq!(
        h.send("GET", "/snapshot", None, None, false).await.status(),
        401
    );
    let session = body(h.send("GET", "/session", None, None, false).await).await;
    assert_eq!(session["authenticated"], false);
    let req = Request::builder()
        .method("POST")
        .uri(format!("{API}/tasks"))
        .header("host", "127.0.0.1:4311")
        .header("origin", "http://127.0.0.1:4311")
        .header("cookie", &h.cookie)
        .header("content-type", "application/json")
        .header("idempotency-key", "no-csrf-key")
        .body(Body::from("{}"))
        .unwrap();
    assert_eq!(
        body(h.app.router().oneshot(req).await.unwrap()).await["error"],
        "csrf_rejected"
    );
    for (host, origin) in [
        ("evil.test:4311", "http://evil.test:4311"),
        ("127.0.0.1:4311", "http://evil.test"),
    ] {
        let req = Request::builder()
            .method("POST")
            .uri(format!("{API}/session"))
            .header("host", host)
            .header("origin", origin)
            .header("content-type", "application/json")
            .body(Body::from(json!({"key":KEY}).to_string()))
            .unwrap();
        assert_eq!(h.app.router().oneshot(req).await.unwrap().status(), 403);
    }
    let logout = h
        .send("DELETE", "/session", Some(json!({})), None, true)
        .await;
    assert_eq!(logout.status(), 200);
    assert_eq!(
        h.send("GET", "/snapshot", None, None, true).await.status(),
        401
    );
}

#[tokio::test]
async fn login_failures_are_bounded() {
    let h = Harness::new().await;
    for _ in 0..10 {
        assert_eq!(
            h.send(
                "POST",
                "/session",
                Some(json!({"key":"wrong"})),
                None,
                false
            )
            .await
            .status(),
            401
        );
    }
    assert_eq!(
        h.send("POST", "/session", Some(json!({"key":KEY})), None, false)
            .await
            .status(),
        429
    );
    // Existing authenticated sessions remain usable during the login cooldown.
    assert_eq!(
        h.send("GET", "/snapshot", None, None, true).await.status(),
        200
    );
}

#[tokio::test]
async fn persistence_revisions_and_idempotency_survive_restart() {
    let h = Harness::new().await;
    let created = h.create("engineering").await;
    let path = format!("/tasks/{}", created["id"].as_str().unwrap());
    let edit = json!({"expected_revision":1,"title":"Revised outcome","status":"ready"});
    let response = h
        .command(&path, "PATCH", edit.clone(), "edit-task", 200)
        .await;
    assert_eq!(response["task"]["revision"], 2);
    assert_eq!(
        h.command(&path, "PATCH", edit.clone(), "edit-task", 200)
            .await,
        response
    );
    assert_eq!(
        h.command(
            &path,
            "PATCH",
            json!({"expected_revision":2,"title":"different"}),
            "edit-task",
            409
        )
        .await["error"],
        "idempotency_conflict"
    );
    assert_eq!(
        h.command(
            &path,
            "PATCH",
            json!({"expected_revision":1,"title":"stale"}),
            "stale-task",
            409
        )
        .await["error"],
        "revision_conflict"
    );
    let db = h.root.path().join("board.sqlite");
    let Harness {
        app,
        root,
        cookie: _,
        csrf: _,
    } = h;
    drop(app);
    let app = App::open(&db, KEY, 4311).unwrap();
    let mut h = Harness {
        app,
        root,
        cookie: String::new(),
        csrf: String::new(),
    };
    assert_eq!(
        h.send("GET", "/snapshot", None, None, true).await.status(),
        401
    );
    h.login().await;
    assert_eq!(
        h.command(&path, "PATCH", edit, "edit-task", 200).await,
        response
    );
    let snapshot = body(h.send("GET", "/snapshot", None, None, true).await).await;
    assert_eq!(snapshot["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(snapshot["tasks"][0]["title"], "Revised outcome");
    assert_eq!(snapshot["events"].as_array().unwrap().len(), 2);
    assert_eq!(snapshot["watermark"], 2);
}

#[tokio::test]
async fn engineering_repository_and_execution_authority_fail_closed() {
    let h = Harness::new().await;
    for (index, repo) in [
        Value::Null,
        json!("https://github.com/owner/repository"),
        json!("owner/repo/path"),
        json!("owner/.."),
        json!("../repo"),
        json!("owner/repo.git"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(h.command("/tasks","POST",json!({"title":"Invalid","objective":"Outcome","workflow":"engineering","repository":repo}),&format!("invalid-repo-{index}"),400).await["error"],"invalid_repository");
    }
    let task = h.create("engineering").await;
    let path = format!("/tasks/{}", task["id"].as_str().unwrap());
    for status in ["active", "review", "done"] {
        assert_eq!(
            h.command(
                &path,
                "PATCH",
                json!({"expected_revision":1,"status":status}),
                &format!("invalid-state-{status}"),
                503
            )
            .await["error"],
            "execution_unavailable"
        );
    }
    assert_eq!(
        h.command(
            &format!("{path}/run"),
            "POST",
            json!({"expected_revision":1}),
            "no-worker-run",
            503
        )
        .await["error"],
        "execution_unavailable"
    );
    assert_eq!(
        h.command(
            &format!("{path}/accept"),
            "POST",
            json!({"expected_revision":1}),
            "no-worker-accept",
            503
        )
        .await["error"],
        "execution_unavailable"
    );
    assert_eq!(
        h.command(
            &path,
            "PATCH",
            json!({"expected_revision":1,"automation_policy":"ready_to_done"}),
            "no-automation",
            503
        )
        .await["error"],
        "automation_unavailable"
    );
    assert_eq!(
        h.command(
            &path,
            "PATCH",
            json!({"expected_revision":1,"workflow":"manual","repository":null,"status":"done"}),
            "no-workflow-bypass",
            409
        )
        .await["error"],
        "workflow_locked"
    );
    let snapshot = body(h.send("GET", "/snapshot", None, None, true).await).await;
    assert_eq!(snapshot["tasks"][0]["revision"], 1);
    assert_eq!(snapshot["runs"], json!([]));
    for capability in ["run", "stop", "ready_to_done"] {
        assert_eq!(snapshot["capabilities"][capability], false);
    }
    assert_eq!(snapshot["watermark"], 1);
}

#[tokio::test]
async fn manual_completion_archival_restore_and_reopen_preserve_history() {
    let h = Harness::new().await;
    let task = h.create("manual").await;
    let path = format!("/tasks/{}", task["id"].as_str().unwrap());
    assert_eq!(
        h.command(
            &format!("{path}/archive"),
            "POST",
            json!({"expected_revision":1}),
            "early-archive",
            409
        )
        .await["error"],
        "task_not_closed"
    );
    h.command(
        &format!("{path}/block"),
        "POST",
        json!({"expected_revision":1,"blocked_reason":"Waiting for input"}),
        "block-task",
        200,
    )
    .await;
    assert_eq!(
        h.command(
            &format!("{path}/accept"),
            "POST",
            json!({"expected_revision":2}),
            "blocked-accept",
            409
        )
        .await["error"],
        "task_not_acceptable"
    );
    h.command(
        &format!("{path}/block"),
        "POST",
        json!({"expected_revision":2,"blocked_reason":null}),
        "unblock-task",
        200,
    )
    .await;
    let done = h
        .command(
            &format!("{path}/accept"),
            "POST",
            json!({"expected_revision":3}),
            "accept-task",
            200,
        )
        .await["task"]
        .clone();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let archived = h
        .command(
            &format!("{path}/archive"),
            "POST",
            json!({"expected_revision":4}),
            "archive-task",
            200,
        )
        .await;
    assert!(!archived["task"]["archived_at"].is_null());
    assert_eq!(archived["task"]["completed_at"], done["completed_at"]);
    let restored = h
        .command(
            &format!("{path}/restore"),
            "POST",
            json!({"expected_revision":5}),
            "restore-task",
            200,
        )
        .await;
    assert_eq!(restored["task"]["completed_at"], done["completed_at"]);
    assert!(restored["task"]["archived_at"].is_null());
    let reopened = h
        .command(
            &format!("{path}/reopen"),
            "POST",
            json!({"expected_revision":6}),
            "reopen-task",
            200,
        )
        .await;
    assert_eq!(reopened["task"]["status"], "backlog");
    assert!(reopened["task"]["completed_at"].is_null());
}

#[tokio::test]
async fn inputs_are_bounded_and_unknown_fields_cannot_forge_state() {
    let h = Harness::new().await;
    for (index,input) in [json!({"title":"Task","objective":""}),json!({"title":"x".repeat(241),"objective":"Outcome"}),json!({"title":"Task","objective":"Outcome","priority":10}),json!({"title":"Task","objective":"Outcome","revision":99}),json!({"title":"Task","objective":"Outcome","github_issue_url":"https://github.com/o/r/issues/0"}),json!({"title":"Task","objective":"Outcome","github_pull_request_url":"https://evil.example/o/r/pull/1"})].into_iter().enumerate() {
        h.command("/tasks","POST",input,&format!("invalid-input-{index}"),400).await;
    }
    let large = json!({"title":"Task","objective":"x".repeat(33000)});
    h.command("/tasks", "POST", large, "large-payload", 413)
        .await;
    let snapshot = body(h.send("GET", "/snapshot", None, None, true).await).await;
    assert_eq!(snapshot["tasks"], json!([]));
}

#[tokio::test]
async fn events_replay_cursor_and_notify_without_polling() {
    let h = Harness::new().await;
    let task = h.create("manual").await;
    let response = h.send("GET", "/events?after=0", None, None, true).await;
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut stream = response.into_body();
    let first = stream.frame().await.unwrap().unwrap().into_data().unwrap();
    let first = String::from_utf8(first.to_vec()).unwrap();
    assert!(first.contains("id: 1"));
    assert!(first.contains("event: workboard"));
    assert!(first.contains("task.created"));
    drop(stream);
    let mut stream = h
        .send("GET", "/events?after=1", None, None, true)
        .await
        .into_body();
    let pending =
        tokio::spawn(async move { stream.frame().await.unwrap().unwrap().into_data().unwrap() });
    let path = format!("/tasks/{}", task["id"].as_str().unwrap());
    h.command(
        &path,
        "PATCH",
        json!({"expected_revision":1,"status":"ready"}),
        "ready-notify",
        200,
    )
    .await;
    let next = tokio::time::timeout(std::time::Duration::from_secs(2), pending)
        .await
        .unwrap()
        .unwrap();
    let next = String::from_utf8(next.to_vec()).unwrap();
    assert!(next.contains("id: 2"));
    assert!(next.contains("task.updated"));
    assert_eq!(
        h.send("GET", "/events?after=-1", None, None, true)
            .await
            .status(),
        400
    );
}

#[tokio::test]
async fn logout_invalidates_open_event_stream() {
    let h = Harness::new().await;
    let mut stream = h
        .send("GET", "/events?after=0", None, None, true)
        .await
        .into_body();
    let pending =
        tokio::spawn(async move { stream.frame().await.unwrap().unwrap().into_data().unwrap() });
    assert_eq!(
        h.send("DELETE", "/session", Some(json!({})), None, true)
            .await
            .status(),
        200
    );
    let frame = tokio::time::timeout(std::time::Duration::from_secs(2), pending)
        .await
        .unwrap()
        .unwrap();
    assert!(
        String::from_utf8(frame.to_vec())
            .unwrap()
            .contains("event: session_expired")
    );
}

#[test]
fn database_refuses_foreign_files_and_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("foreign.sqlite");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("CREATE TABLE unrelated(value TEXT)", [])
        .unwrap();
    drop(db);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let before = std::fs::read(&path).unwrap();
    assert_eq!(
        App::open(&path, KEY, 4311).err().unwrap().code(),
        "database_not_workboard_rs"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let empty = root.path().join("empty-foreign.sqlite");
    let db = rusqlite::Connection::open(&empty).unwrap();
    db.execute_batch("PRAGMA application_id=12345;").unwrap();
    drop(db);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&empty, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert_eq!(
        App::open(&empty, KEY, 4311).err().unwrap().code(),
        "database_not_workboard_rs"
    );
    #[cfg(unix)]
    {
        let target = root.path().join("must-not-create.sqlite");
        let link = root.path().join("dangling.sqlite");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(
            App::open(&link, KEY, 4311).err().unwrap().code(),
            "database_path_invalid"
        );
        assert!(!target.exists());
        let private = root.path().join("private.sqlite");
        let _app = App::open(&private, KEY, 4311).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(private).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn shutdown_closes_event_streams() {
    let h = Harness::new().await;
    let stream = h
        .send("GET", "/events?after=0", None, None, true)
        .await
        .into_body();
    let pending = tokio::spawn(async move { stream.collect().await.unwrap().to_bytes() });
    h.app.shutdown();
    let bytes = tokio::time::timeout(std::time::Duration::from_secs(2), pending)
        .await
        .unwrap()
        .unwrap();
    assert!(String::from_utf8(bytes.to_vec()).unwrap().is_empty());
}

#[tokio::test]
async fn unicode_task_fields_roundtrip_with_character_limits() {
    let h = Harness::new().await;
    let title = "é".repeat(240);
    let owner = "李".repeat(160);
    let created = h
        .command(
            "/tasks",
            "POST",
            json!({"title": title, "objective": "完成可验证的任务", "owner": owner}),
            "unicode-task",
            201,
        )
        .await;
    let path = format!("/tasks/{}", created["task"]["id"].as_str().unwrap());
    let stored = body(h.send("GET", &path, None, None, true).await).await;
    assert_eq!(stored["task"]["title"], title);
    assert_eq!(stored["task"]["owner"], owner);
    assert_eq!(
        h.command(
            &path,
            "PATCH",
            json!({"expected_revision": 1, "title": "é".repeat(241)}),
            "unicode-oversize",
            400
        )
        .await["error"],
        "invalid_task_text"
    );
}

#[tokio::test]
async fn ticket_metadata_defaults_normalizes_and_dependencies_reject_missing_self_and_cycles() {
    let h = Harness::new().await;
    let a=h.command("/tasks","POST",json!({"title":"Research","objective":"Investigate","task_type":"research","labels":["  User Experience ","USER-experience","QA"],"workflow":"manual"}),"research",201).await["task"].clone();
    assert_eq!(a["qa_policy"], "skip");
    assert_eq!(a["labels"], json!(["user-experience", "qa"]));
    let b=h.command("/tasks","POST",json!({"title":"Required research","objective":"Investigate safely","task_type":"research","qa_policy":"required","workflow":"manual","dependencies":[a["id"]]}),"required-research",201).await["task"].clone();
    assert_eq!(b["qa_policy"], "required");
    assert_eq!(b["dependency_blockers"][0]["id"], a["id"]);
    let path = format!("/tasks/{}", a["id"].as_str().unwrap());
    for (key, deps, code) in [
        ("self", json!([a["id"]]), "dependency_cycle"),
        ("cycle", json!([b["id"]]), "dependency_cycle"),
        (
            "missing",
            json!([uuid::Uuid::new_v4().to_string()]),
            "dependency_not_found",
        ),
        (
            "duplicates",
            json!([b["id"], b["id"]]),
            "invalid_dependencies",
        ),
    ] {
        let v = h
            .command(
                &path,
                "PATCH",
                json!({"expected_revision":a["revision"],"dependencies":deps}),
                &format!("dependency-{key}"),
                400,
            )
            .await;
        assert_eq!(v["error"], code);
    }
    let edited = h
        .command(
            &format!("/tasks/{}", b["id"].as_str().unwrap()),
            "PATCH",
            json!({"expected_revision":b["revision"],"task_type":"chore"}),
            "change-type",
            200,
        )
        .await;
    assert_eq!(edited["task"]["qa_policy"], "required");
    h.command(
        &format!("{path}/accept"),
        "POST",
        json!({"expected_revision":a["revision"]}),
        "complete-parent",
        200,
    )
    .await;
    let snapshot = body(h.send("GET", "/snapshot", None, None, true).await).await;
    let child = snapshot["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == b["id"])
        .unwrap();
    assert_eq!(child["dependency_blockers"], json!([]));
    let legacy = h.create("manual").await;
    assert_eq!(legacy["task_type"], "feature");
    assert_eq!(legacy["qa_policy"], "required");
}

#[tokio::test]
async fn assembled_task_size_rejects_accumulating_valid_patches_atomically() {
    let h = Harness::new().await;
    let task = h.create("engineering").await;
    let path = format!("/tasks/{}", task["id"].as_str().unwrap());
    let context = json!({"expected_revision":task["revision"],"context":vec!["c".repeat(1900);16]});
    assert!(context.to_string().len() < 32768);
    let first = h
        .command(&path, "PATCH", context, "large-context", 200)
        .await["task"]
        .clone();
    let criteria = json!({"expected_revision":first["revision"],"acceptance_criteria":vec!["a".repeat(1900);16]});
    assert!(criteria.to_string().len() < 32768);
    let refused = h
        .command(&path, "PATCH", criteria.clone(), "large-criteria", 400)
        .await;
    assert_eq!(refused["error"], "task_too_large");
    let unchanged = body(h.send("GET", &path, None, None, true).await).await["task"].clone();
    assert_eq!(unchanged, first);
    h.command(&path,"PATCH",json!({"expected_revision":first["revision"],"context":["Concise background"],"acceptance_criteria":["A focused acceptance test"]}),"reduce-task",200).await;
}

fn restart_browser(h: Harness, key: &str) -> Harness {
    let Harness {
        app,
        root,
        cookie,
        csrf,
    } = h;
    app.shutdown();
    drop(app);
    let app = App::open(&root.path().join("board.sqlite"), key, 4311).unwrap();
    Harness {
        app,
        root,
        cookie,
        csrf,
    }
}
#[tokio::test]
async fn remembered_browser_survives_restart_and_logout_and_rotation_are_durable() {
    let h = Harness::new().await;
    assert!(h.cookie.starts_with("workboard_rs_session_4311="));
    let csrf = h.csrf.clone();
    let h = restart_browser(h, KEY);
    let session = body(h.send("GET", "/session", None, None, true).await).await;
    assert_eq!(session["authenticated"], true);
    assert_eq!(session["csrf_token"], csrf);
    h.command(
        "/tasks",
        "POST",
        json!({"title":"Remembered operator","objective":"Mutations retain CSRF after restart"}),
        "remembered-create",
        201,
    )
    .await;
    let logout = h
        .send("DELETE", "/session", Some(json!({})), None, true)
        .await;
    assert_eq!(logout.status(), 200);
    assert!(
        logout.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .starts_with("workboard_rs_session_4311=;")
    );
    assert!(
        logout.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    let mut h = restart_browser(h, KEY);
    assert_eq!(
        body(h.send("GET", "/session", None, None, true).await).await["authenticated"],
        false
    );
    h.login().await;
    let h = restart_browser(h, "new-operator-key-012345678901234567890");
    assert_eq!(
        h.send("GET", "/snapshot", None, None, true).await.status(),
        401
    );
    let h = restart_browser(h, KEY);
    assert_eq!(
        body(h.send("GET", "/session", None, None, true).await).await["authenticated"],
        false
    );
}
#[tokio::test]
async fn remembered_cookie_ports_coexist_and_duplicate_active_cookies_fail_closed() {
    let h = Harness::new().await;
    let other = tempfile::tempdir().unwrap();
    let app = App::open(&other.path().join("board.sqlite"), KEY, 4315).unwrap();
    let response = app
        .router()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("{API}/session"))
                .header("host", "127.0.0.1:4315")
                .header("origin", "http://127.0.0.1:4315")
                .header("content-type", "application/json")
                .body(Body::from(json!({"key":KEY}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let cookie = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    assert!(cookie.starts_with("workboard_rs_session_4315="));
    assert_ne!(cookie, h.cookie);
    for (app, port) in [(&h.app, 4311), (&app, 4315)] {
        let response = app
            .router()
            .oneshot(
                Request::builder()
                    .uri(format!("{API}/session"))
                    .header("host", format!("127.0.0.1:{port}"))
                    .header("cookie", format!("{}; {cookie}", h.cookie))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(body(response).await["authenticated"], true);
    }
    let response = h
        .app
        .router()
        .oneshot(
            Request::builder()
                .uri(format!("{API}/snapshot"))
                .header("host", "127.0.0.1:4311")
                .header("cookie", &h.cookie)
                .header("cookie", &h.cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let db = rusqlite::Connection::open(h.root.path().join("board.sqlite")).unwrap();
    let token = h.cookie.split_once('=').unwrap().1;
    let hash: String = db
        .query_row("SELECT token_hash FROM operator_sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(hash.len(), 64);
    assert_ne!(hash, token);
    let columns: Vec<String> = db
        .prepare("PRAGMA table_info(operator_sessions)")
        .unwrap()
        .query_map([], |r| r.get(1))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(
        !columns
            .iter()
            .any(|c| c.contains("csrf") || c == "token" || c == "key")
    );
}
#[tokio::test]
async fn daily_session_renewal_reissues_cookie_and_failed_logout_never_claims_revocation() {
    let h = Harness::new().await;
    let db = rusqlite::Connection::open(h.root.path().join("board.sqlite")).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let prior = now - 86401000;
    db.execute(
        "UPDATE operator_sessions SET issued_ms=?1,renewed_ms=?1,expires_ms=?2",
        rusqlite::params![prior, prior + 34560000000i64],
    )
    .unwrap();
    let renewed = h.send("GET", "/session", None, None, true).await;
    assert_eq!(renewed.status(), 200);
    let cookie = renewed.headers()["set-cookie"].to_str().unwrap();
    assert!(cookie.contains("Max-Age=34560000"));
    assert!(cookie.starts_with(&h.cookie));
    let session = body(renewed).await;
    assert_eq!(session["csrf_token"], h.csrf);
    assert!(session["expires_at"].as_i64().unwrap() > prior + 34560000000i64);
    db.execute_batch("CREATE TRIGGER synthetic_revoke_failure BEFORE DELETE ON operator_sessions BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    let refused = h
        .send("DELETE", "/session", Some(json!({})), None, true)
        .await;
    assert_eq!(refused.status(), 503);
    assert!(refused.headers().get("set-cookie").is_none());
    assert_eq!(
        h.send("GET", "/snapshot", None, None, true).await.status(),
        200
    );
    db.execute_batch("DROP TRIGGER synthetic_revoke_failure;")
        .unwrap();
    assert_eq!(
        h.send("DELETE", "/session", Some(json!({})), None, true)
            .await
            .status(),
        200
    );
}

#[tokio::test]
async fn system_registry_atomic_crud_conflicts_permissions_and_restart() {
    let h = Harness::new().await;
    let initial = body(h.send("GET", "/snapshot", None, None, true).await).await["system"].clone();
    assert_eq!(
        initial["defaults"],
        json!({"main":"main","coder":"coder","qa":"qa"})
    );
    assert_eq!(initial["agents"].as_array().unwrap().len(), 3);
    let mut update = json!({"expected_revision":initial["revision"],"agents":initial["agents"],"defaults":initial["defaults"]});
    let custom = json!({"id":"research-coder","label":"Research coder","role":"coder","instructions":"Use bounded assertions and disclose limitations.","model":"reviewed-model","enabled":true});
    update["agents"]
        .as_array_mut()
        .unwrap()
        .push(custom.clone());
    update["defaults"]["coder"] = custom["id"].clone();
    let unauthed = h
        .send(
            "PUT",
            "/system/agents",
            Some(update.clone()),
            Some("no-auth-agent"),
            false,
        )
        .await;
    assert_eq!(unauthed.status(), 401);
    let saved = h
        .command("/system/agents", "PUT", update.clone(), "save-system", 200)
        .await;
    assert_eq!(saved["system"]["revision"], 2);
    assert_eq!(
        h.command("/system/agents", "PUT", update.clone(), "save-system", 200)
            .await,
        saved
    );
    let mut conflict = update.clone();
    conflict["agents"][0]["label"] = json!("Changed");
    assert_eq!(
        h.command("/system/agents", "PUT", conflict, "save-system", 409)
            .await["error"],
        "idempotency_conflict"
    );
    assert_eq!(
        h.command("/system/agents", "PUT", update, "stale-system", 409)
            .await["error"],
        "revision_conflict"
    );
    let valid = json!({"expected_revision":2,"agents":saved["system"]["agents"],"defaults":saved["system"]["defaults"]});
    for (index, payload) in [
        json!({"read_roots":["/"]}),
        json!({"executable":"/bin/sh"}),
        json!({"permissions":"all"}),
    ]
    .into_iter()
    .enumerate()
    {
        let mut bad = valid.clone();
        bad["agents"][0]
            .as_object_mut()
            .unwrap()
            .extend(payload.as_object().unwrap().clone());
        assert_eq!(
            h.command(
                "/system/agents",
                "PUT",
                bad,
                &format!("injection-{index}"),
                400
            )
            .await["error"],
            "agent_registry_invalid"
        );
    }
    let mut bad = valid.clone();
    bad["defaults"]["qa"] = bad["defaults"]["coder"].clone();
    assert_eq!(
        h.command("/system/agents", "PUT", bad, "role-veto", 409)
            .await["error"],
        "agent_unavailable"
    );
    let mut disabled = valid.clone();
    disabled["agents"][3]["enabled"] = json!(false);
    let disabled = h
        .command(
            "/system/agents",
            "PUT",
            disabled,
            "disable-system-coder",
            200,
        )
        .await;
    let Harness {
        app,
        root,
        cookie,
        csrf: _,
    } = h;
    app.shutdown();
    drop(app);
    let reopened = App::open(&root.path().join("board.sqlite"), KEY, 4311).unwrap();
    let request = Request::builder()
        .uri(format!("{API}/snapshot"))
        .header("host", "127.0.0.1:4311")
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap();
    let current = body(reopened.router().oneshot(request).await.unwrap()).await;
    assert_eq!(current["system"], disabled["system"]);
    assert!(current["runs"].as_array().unwrap().is_empty());
}
