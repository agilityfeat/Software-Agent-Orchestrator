use rusqlite::OptionalExtension;
pub mod adapters;
pub mod automation;
pub mod dev;
mod diagnostics;
mod execution;
pub mod execution_config;
mod execution_store;
pub mod offline;
mod operator_auth;
pub mod ownership;
pub mod planning;
pub mod projects;
pub mod qa_feedback;
mod storage_ownership;
mod store;
pub mod system;
pub mod tickets;

use axum::{
    Json, Router,
    body::to_bytes,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{any, get},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    convert::Infallible,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use uuid::Uuid;

const PREFIX: &str = "/api/workboard/v1";
pub type Result<T> = std::result::Result<T, ApiError>;

#[derive(Debug)]
pub struct ApiError {
    status: u16,
    code: &'static str,
    diagnostic: Option<diagnostics::Failure>,
}
impl ApiError {
    pub(crate) fn new(status: u16, code: &'static str) -> Self {
        Self {
            status,
            code,
            diagnostic: None,
        }
    }
    pub(crate) fn bad(code: &'static str) -> Self {
        Self::new(400, code)
    }
    pub(crate) fn conflict(code: &'static str) -> Self {
        Self::new(409, code)
    }
    pub(crate) fn unavailable(code: &'static str) -> Self {
        Self::new(503, code)
    }
    pub(crate) fn internal() -> Self {
        Self::unavailable("workboard_unavailable")
    }
    pub(crate) fn with_diagnostic(mut self, value: diagnostics::Failure) -> Self {
        self.diagnostic = Some(value);
        self
    }
    pub(crate) fn diagnostic(&self) -> Option<diagnostics::Failure> {
        self.diagnostic
            .clone()
            .map(|d| d.with_code(self.code))
            .or_else(|| diagnostics::Failure::for_code(self.code))
    }
    /// Prints only a strict typed diagnostic, never subprocess stderr.
    pub fn report_failure(&self) {
        if let Some(value) = self.diagnostic() {
            eprintln!("{}", value.json());
        } else {
            eprintln!("{}", self.code);
        }
    }
    pub fn code(&self) -> &'static str {
        self.code
    }
}
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code)
    }
}
impl std::error::Error for ApiError {}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(json!({"error":self.code})),
        )
            .into_response()
    }
}

struct Inner {
    store: Mutex<store::Store>,
    auth: operator_auth::Auth,
    key_hash: [u8; 32],
    authority: String,
    changes: tokio::sync::watch::Sender<u64>,
    failed_logins: Mutex<VecDeque<Instant>>,
    streams: Arc<tokio::sync::Semaphore>,
    approval: Option<execution_config::Approval>,
    ownership: Option<Mutex<ownership::Guard>>,
    automation: Option<automation::Activation>,
    factory: Option<projects::Factory>,
    wake: tokio::sync::Notify,
    stopping: AtomicBool,
}
#[derive(Clone)]
pub struct App {
    inner: Arc<Inner>,
}

impl App {
    /// Creates isolated board storage and a loopback-only HTTP application.
    pub fn open(path: &Path, operator_key: &str, port: u16) -> Result<Self> {
        Self::open_approved(path, operator_key, port, None)
    }
    pub fn open_approved(
        path: &Path,
        operator_key: &str,
        port: u16,
        approval: Option<execution_config::Approval>,
    ) -> Result<Self> {
        Self::open_approved_with_automation(
            path,
            operator_key,
            port,
            approval,
            automation::Activation::from_env(),
        )
    }
    pub fn open_approved_with_automation(
        path: &Path,
        operator_key: &str,
        port: u16,
        approval: Option<execution_config::Approval>,
        automation: Option<automation::Activation>,
    ) -> Result<Self> {
        Self::open_runtime(
            path,
            operator_key,
            port,
            approval,
            automation,
            projects::Factory::from_env()?,
        )
    }
    pub fn open_projects(
        path: &Path,
        key: &str,
        port: u16,
        factory: projects::Factory,
    ) -> Result<Self> {
        Self::open_runtime(path, key, port, None, None, Some(factory))
    }
    fn open_runtime(
        path: &Path,
        operator_key: &str,
        port: u16,
        approval: Option<execution_config::Approval>,
        automation: Option<automation::Activation>,
        factory: Option<projects::Factory>,
    ) -> Result<Self> {
        if !(32..=256).contains(&operator_key.len())
            || operator_key.chars().any(char::is_control)
            || port == 0
        {
            return Err(ApiError::bad("configuration_invalid"));
        }
        if factory.is_some()
            && approval
                .as_ref()
                .is_some_and(|a| a.policy.ownership.is_some())
        {
            return Err(ApiError::bad("project_setup_mixed_ownership"));
        }
        let mut store = store::Store::open(path)?;
        system::initialize(&store.db, factory.as_ref().and_then(|f| f.native_agents()))?;
        let key_hash: [u8; 32] = Sha256::digest(operator_key.as_bytes()).into();
        let auth = operator_auth::Auth::open(&mut store.db, path, key_hash, port)?;
        let ownership = approval
            .as_ref()
            .and_then(|a| a.policy.ownership.as_ref())
            .map(|b| ownership::Guard::open(b, path).map(Mutex::new))
            .transpose()?;
        Ok(Self {
            inner: Arc::new(Inner {
                store: Mutex::new(store),
                ownership,
                automation,
                factory,
                auth,
                key_hash,
                authority: format!("127.0.0.1:{port}"),
                changes: tokio::sync::watch::channel(0).0,
                failed_logins: Mutex::new(VecDeque::new()),
                streams: Arc::new(tokio::sync::Semaphore::new(64)),
                approval,
                wake: tokio::sync::Notify::new(),
                stopping: AtomicBool::new(false),
            }),
        })
    }
    /// Ends authenticated streams before the listener begins graceful shutdown.
    pub fn changed(&self) {
        self.inner
            .changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
        self.inner.wake.notify_one();
    }
    fn capabilities(&self) -> Value {
        let state = self
            .inner
            .store
            .lock()
            .ok()
            .and_then(|s| s.execution_state().ok())
            .unwrap_or(json!({"paused":true}));
        let cleanup = self
            .inner
            .approval
            .as_ref()
            .is_some_and(|a| a.verify(&a.digest, true).is_ok());
        let owned = self
            .inner
            .approval
            .as_ref()
            .and_then(|a| a.policy.ownership.as_ref())
            .is_none_or(|b| self.assert_ownership(&b.repository).is_ok());
        let approved = cleanup
            && owned
            && self
                .inner
                .approval
                .as_ref()
                .is_some_and(|a| a.policy.expires_at_ms > execution_config::now_ms());
        let projects_ready = self
            .inner
            .factory
            .as_ref()
            .is_some_and(|f| f.verify().is_ok());
        let available = approved || projects_ready;
        let paused = state["paused"] != false;
        let busy = !state["lane_run_id"].is_null();
        json!({"tasks":true,"write":true,"accept":true,"run":available && !paused && !busy,"cancel":cleanup||self.inner.factory.is_some(),"stop":cleanup||self.inner.factory.is_some(),"projects":projects_ready,"planning":self.inner.factory.as_ref().is_some_and(|f|f.planning_available()),"ready_to_done":projects_ready||self.automation_available(),"paused":paused,"execution_state":if !available {"unavailable"} else if busy {"busy"} else if paused {"paused"} else {"ready"}})
    }
    pub(crate) fn automation_available(&self) -> bool {
        let Some(activation) = &self.inner.automation else {
            return false;
        };
        let Some(approval) = &self.inner.approval else {
            return false;
        };
        activation.verify(approval).is_ok()
            && self.assert_ownership(activation.repository()).is_ok()
            && self.inner.store.lock().ok().is_some_and(|s| {
                s.db.query_row(
                    "SELECT count(*) FROM automation_consumptions WHERE activation_sha256=?1",
                    [&activation.digest],
                    |r| r.get::<_, i64>(0),
                )
                .is_ok_and(|n| n == 0)
            })
    }
    pub(crate) fn assert_automatic_run(&self, run: &Value) -> Result<()> {
        if run["project_id"].is_string() {
            self.inner
                .factory
                .as_ref()
                .ok_or_else(|| ApiError::unavailable("project_runtime_unavailable"))?
                .verify()?;
            return Ok(());
        }
        if let Some(digest) = run["automation"]["activation_sha256"].as_str() {
            let activation = self
                .inner
                .automation
                .as_ref()
                .ok_or_else(|| ApiError::unavailable("automation_unavailable"))?;
            if digest != activation.digest {
                return Err(ApiError::unavailable("automation_unavailable"));
            }
            activation.verify(
                self.inner
                    .approval
                    .as_ref()
                    .ok_or_else(|| ApiError::unavailable("execution_unavailable"))?,
            )?;
        }
        Ok(())
    }
    pub(crate) fn assert_ownership(&self, repository: &str) -> Result<()> {
        if let Some(guard) = &self.inner.ownership {
            guard
                .lock()
                .map_err(|_| ApiError::internal())?
                .assert(repository)?;
        }
        Ok(())
    }
    pub(crate) fn task_approval(&self, task: &Value) -> Result<Option<execution_config::Approval>> {
        if let Some(id) = task["project_id"].as_str() {
            let project = {
                let s = self.inner.store.lock().map_err(|_| ApiError::internal())?;
                projects::get(&s.db, id)?
            };
            return self
                .inner
                .factory
                .as_ref()
                .ok_or_else(|| ApiError::unavailable("project_runtime_unavailable"))?
                .approval(&project)
                .map(Some);
        }
        if let Some(factory) = &self.inner.factory {
            let p = {
                let store = self.inner.store.lock().map_err(|_| ApiError::internal())?;
                projects::by_repository(&store.db, task["repository"].as_str().unwrap_or(""))?
            };
            if let Some(p) = p {
                return factory.approval(&p).map(Some);
            }
        }
        Ok(self.inner.approval.clone())
    }
    pub(crate) fn task_run_approval(
        &self,
        task: &Value,
    ) -> Result<Option<execution_config::Approval>> {
        let base = self.task_approval(task)?;
        let Some(factory) = &self.inner.factory else {
            return Ok(base);
        };
        factory.verify()?;
        let registry = {
            let store = self.inner.store.lock().map_err(|_| ApiError::internal())?;
            system::get(&store.db)?
        };
        base.map(|approval| approval.with_agents(registry.assignment(Some(task))?))
            .transpose()
    }
    pub(crate) fn run_approval(&self, run: &Value) -> Result<execution_config::Approval> {
        if run["project_id"].is_string() {
            return self.inner.factory.as_ref().ok_or_else(||ApiError::unavailable("project_runtime_unavailable"))?.approval(&json!({"approval_path":run["runtime_approval_path"],"approval_sha256":run["approval_sha256"]}));
        }
        self.inner
            .approval
            .clone()
            .ok_or_else(|| ApiError::unavailable("execution_unavailable"))
    }
    pub fn finish_ownership(&self) -> Result<()> {
        let store = self.inner.store.lock().map_err(|_| ApiError::internal())?;
        if let Some(guard) = &self.inner.ownership {
            guard
                .lock()
                .map_err(|_| ApiError::internal())?
                .finish(&store.db)?;
        }
        Ok(())
    }
    pub fn shutdown(&self) {
        self.inner.stopping.store(true, Ordering::SeqCst);
        self.inner.wake.notify_one();
        self.inner
            .changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }
    pub fn router(&self) -> Router {
        Router::new()
            .route("/", get(index))
            .route("/index.html", get(index))
            .route("/workboard", get(index))
            .route(
                "/workboard.js",
                get(|| async {
                    asset(
                        "text/javascript; charset=utf-8",
                        include_str!("../public/workboard.js"),
                    )
                }),
            )
            .route(
                "/workboard-model.js",
                get(|| async {
                    asset(
                        "text/javascript; charset=utf-8",
                        include_str!("../public/workboard-model.js"),
                    )
                }),
            )
            .route(
                "/styles.css",
                get(|| async {
                    asset(
                        "text/css; charset=utf-8",
                        include_str!("../public/styles.css"),
                    )
                }),
            )
            .route("/api/workboard/v1/{*path}", any(api))
            .fallback(|| async { ApiError::new(404, "not_found") })
            .layer(middleware::from_fn_with_state(self.clone(), security))
            .with_state(self.clone())
    }
    fn session_id(&self, headers: &HeaderMap) -> Result<Option<String>> {
        let store = self.inner.store.lock().map_err(|_| ApiError::internal())?;
        self.inner
            .auth
            .read(&store.db, headers, execution_config::now_ms())
    }
    fn authorized(&self, headers: &HeaderMap) -> Result<String> {
        self.session_id(headers)?
            .ok_or_else(|| ApiError::new(401, "unauthorized"))
    }
    fn csrf(&self, headers: &HeaderMap, id: &str) -> Result<()> {
        let session = {
            let store = self.inner.store.lock().map_err(|_| ApiError::internal())?;
            self.inner
                .auth
                .session(&store.db, id, execution_config::now_ms())?
        };
        let supplied = headers
            .get("x-csrf-token")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if session.is_some_and(|s| bool::from(s.csrf.as_bytes().ct_eq(supplied.as_bytes()))) {
            Ok(())
        } else {
            Err(ApiError::new(403, "csrf_rejected"))
        }
    }
    fn session_document(&self, id: Option<&str>) -> Result<Value> {
        // Do not hold the auth/store lock while capabilities takes its own store lock.
        let session = {
            let store = self.inner.store.lock().map_err(|_| ApiError::internal())?;
            id.map(|id| {
                self.inner
                    .auth
                    .session(&store.db, id, execution_config::now_ms())
            })
            .transpose()?
            .flatten()
        };
        Ok(match session {
            Some(s) => {
                json!({"authenticated":true,"csrf_token":s.csrf,"expires_at":s.expires,"identity":"Local operator","login_method":"key","capabilities":self.capabilities()})
            }
            None => {
                json!({"authenticated":false,"login_method":"key","capabilities":{"tasks":false,"write":false,"run":false,"stop":false,"accept":false,"ready_to_done":false}})
            }
        })
    }
    fn renew_session(&self, headers: &HeaderMap) -> Result<Option<String>> {
        let mut store = self.inner.store.lock().map_err(|_| ApiError::internal())?;
        self.inner
            .auth
            .renew(&mut store.db, headers, execution_config::now_ms())
    }
}

fn asset(content_type: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type)], body).into_response()
}
async fn index() -> Response {
    asset(
        "text/html; charset=utf-8",
        include_str!("../public/index.html"),
    )
}

async fn security(State(app): State<App>, req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok());
    let localhost = app.inner.authority.replacen("127.0.0.1", "localhost", 1);
    let mut response = if host != Some(app.inner.authority.as_str())
        && host != Some(localhost.as_str())
    {
        ApiError::new(403, "host_not_allowed").into_response()
    } else if req.method() != Method::GET
        && req.method() != Method::HEAD
        && req
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            != host.map(|h| format!("http://{h}")).as_deref()
    {
        ApiError::new(403, "origin_not_allowed").into_response()
    } else if app.inner.stopping.load(Ordering::SeqCst) {
        ApiError::unavailable("service_stopping").into_response()
    } else {
        let renewal =
            if req.uri().path() == format!("{PREFIX}/session") && req.method() != Method::GET {
                Ok(None)
            } else {
                app.renew_session(req.headers())
            };
        match renewal {
            Err(e) => e.into_response(),
            Ok(token) => {
                let mut response = next.run(req).await;
                if !response.headers().contains_key(header::SET_COOKIE) {
                    if let Some(token) = token {
                        if let Ok(cookie) = HeaderValue::from_str(&app.inner.auth.cookie(&token)) {
                            response.headers_mut().insert(header::SET_COOKIE, cookie);
                        }
                    }
                }
                response
            }
        }
    };
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert("content-security-policy",HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"));
    response
}
async fn input(req: Request) -> Result<Value> {
    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if content_type.split(';').next().unwrap_or("").trim() != "application/json" {
        return Err(ApiError::new(415, "unsupported_media_type"));
    }
    let bytes = to_bytes(req.into_body(), 32768)
        .await
        .map_err(|_| ApiError::new(413, "body_too_large"))?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| ApiError::bad("invalid_json"))?;
    if !value.is_object() {
        return Err(ApiError::bad("request_invalid"));
    }
    Ok(value)
}
async fn api(State(app): State<App>, req: Request) -> Response {
    match handle(app, req).await {
        Ok(response) => response,
        Err(err) => err.into_response(),
    }
}
async fn handle(app: App, req: Request) -> Result<Response> {
    let path = req
        .uri()
        .path()
        .strip_prefix(PREFIX)
        .unwrap_or("")
        .to_string();
    let method = req.method().clone();
    let headers = req.headers().clone();
    let session = app.session_id(&headers)?;
    if path == "/session" {
        if method == Method::GET {
            return Ok(Json(app.session_document(session.as_deref())?).into_response());
        }
        if method == Method::POST {
            let body = input(req).await?;
            if body.as_object().is_none_or(|b| b.len() != 1) {
                return Err(ApiError::bad("request_invalid"));
            }
            {
                let mut failures = app
                    .inner
                    .failed_logins
                    .lock()
                    .map_err(|_| ApiError::internal())?;
                while failures
                    .front()
                    .is_some_and(|at| at.elapsed() > Duration::from_secs(60))
                {
                    failures.pop_front();
                }
                if failures.len() >= 10 {
                    return Err(ApiError::new(429, "login_rate_limited"));
                }
            }
            let key = body["key"]
                .as_str()
                .filter(|s| s.len() <= 256)
                .ok_or_else(|| ApiError::new(401, "unauthorized"))?;
            let digest: [u8; 32] = Sha256::digest(key.as_bytes()).into();
            if !bool::from(digest.ct_eq(&app.inner.key_hash)) {
                app.inner
                    .failed_logins
                    .lock()
                    .map_err(|_| ApiError::internal())?
                    .push_back(Instant::now());
                return Err(ApiError::new(401, "unauthorized"));
            }
            let origin = app
                .inner
                .auth
                .origin(&headers)
                .ok_or_else(|| ApiError::new(403, "origin_not_allowed"))?;
            let token = {
                let mut store = app.inner.store.lock().map_err(|_| ApiError::internal())?;
                app.inner.auth.issue(
                    &mut store.db,
                    &origin,
                    session.as_deref(),
                    execution_config::now_ms(),
                )?
            };
            let mut response =
                Json(app.session_document(Some(&operator_auth::hash(&token)))?).into_response();
            response.headers_mut().insert(
                header::SET_COOKIE,
                HeaderValue::from_str(&app.inner.auth.cookie(&token))
                    .map_err(|_| ApiError::internal())?,
            );
            return Ok(response);
        }
        if method == Method::DELETE {
            let id = app.authorized(&headers)?;
            app.csrf(&headers, &id)?;
            {
                let store = app.inner.store.lock().map_err(|_| ApiError::internal())?;
                app.inner.auth.revoke(&store.db, &id)?;
            }
            app.inner
                .changes
                .send_modify(|revision| *revision = revision.wrapping_add(1));
            let mut response = Json(json!({"authenticated":false})).into_response();
            response.headers_mut().insert(
                header::SET_COOKIE,
                HeaderValue::from_str(&app.inner.auth.cookie(""))
                    .map_err(|_| ApiError::internal())?,
            );
            return Ok(response);
        }
        return Err(ApiError::new(405, "method_not_allowed"));
    }
    let id = app.authorized(&headers)?;
    if method == Method::GET {
        if path == "/snapshot" {
            let mut snapshot = app
                .inner
                .store
                .lock()
                .map_err(|_| ApiError::internal())?
                .snapshot()?;
            let registry = {
                let store = app.inner.store.lock().map_err(|_| ApiError::internal())?;
                system::get(&store.db)?
            };
            snapshot["system"] = registry.public();
            snapshot["system"]["editable"] =
                json!(app.inner.factory.is_some() || app.inner.approval.is_none());
            snapshot["coding_agents"] = if app.inner.factory.is_some() {
                registry.coding_agents()
            } else {
                Value::Null
            };
            snapshot["capabilities"] = app.capabilities();
            snapshot["execution"]["approved"] =
                json!(app.inner.approval.is_some() || app.inner.factory.is_some());
            return Ok(Json(snapshot).into_response());
        }
        if path == "/plans" {
            let store = app.inner.store.lock().map_err(|_| ApiError::internal())?;
            return Ok(Json(json!({"plans":planning::list(&store.db)?})).into_response());
        }
        if let Some(id) = path
            .strip_prefix("/plans/")
            .filter(|id| Uuid::parse_str(id).is_ok())
        {
            let store = app.inner.store.lock().map_err(|_| ApiError::internal())?;
            return Ok(
                Json(json!({"plan":planning::public(planning::get(&store.db,id)?)}))
                    .into_response(),
            );
        }
        if path == "/events" {
            let query = req.uri().query().unwrap_or("");
            let raw = headers
                .get("last-event-id")
                .and_then(|v| v.to_str().ok())
                .or_else(|| query.split('&').find_map(|p| p.strip_prefix("after=")))
                .unwrap_or("0");
            if raw.len() > 15 || raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
                return Err(ApiError::bad("cursor_invalid"));
            }
            let mut cursor: i64 = raw.parse().map_err(|_| ApiError::bad("cursor_invalid"))?;
            let permit = app
                .inner
                .streams
                .clone()
                .try_acquire_owned()
                .map_err(|_| ApiError::new(429, "stream_limit_reached"))?;
            let mut changes = app.inner.changes.subscribe();
            let events = async_stream::stream! {
                let _permit=permit;
                loop {
                    if app.inner.stopping.load(Ordering::SeqCst) {break;}
                    match app.session_id(&headers) {Ok(Some(current)) if current==id=>{},Ok(_)=>{yield Ok::<Event,Infallible>(Event::default().event("session_expired").data("{}"));break;},Err(_)=>break}
                    let batch = app.inner.store.lock().map_err(|_|ApiError::internal()).and_then(|store|store.events(cursor,None,false));
                    let Ok(batch)=batch else {break};
                    let full=batch.len()==200;
                    for event in batch {cursor=event["cursor"].as_i64().unwrap_or(cursor);yield Ok::<Event,Infallible>(Event::default().id(cursor.to_string()).event("workboard").data(event.to_string()));}
                    if full {continue;}
                    let _=tokio::time::timeout(Duration::from_secs(30),changes.changed()).await;
                }
            };
            return Ok(Sse::new(events)
                .keep_alive(KeepAlive::new().interval(Duration::from_secs(10)))
                .into_response());
        }
        let pieces: Vec<_> = path.split('/').collect();
        if pieces.len() == 3 && pieces[1] == "tasks" && Uuid::parse_str(pieces[2]).is_ok() {
            let store = app.inner.store.lock().map_err(|_| ApiError::internal())?;
            return Ok(Json(json!({"task":store.task(pieces[2])?,"runs":store.runs(Some(pieces[2]))?,"events":store.events(0,Some(pieces[2]),true)?,"execution_activity":{"state":"available","tasks":{}}})).into_response());
        }
        return Err(ApiError::new(404, "not_found"));
    }
    app.csrf(&headers, &id)?;
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .filter(|v| {
            (8..=128).contains(&v.len())
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
                && !v.starts_with("workboard-maintenance:")
                && !v.starts_with("workboard-automatic:")
        })
        .ok_or_else(|| ApiError::bad("idempotency_key_required"))?
        .to_string();
    let body = input(req).await?;
    let registry = {
        let store = app.inner.store.lock().map_err(|_| ApiError::internal())?;
        system::get(&store.db)?
    };
    let parts: Vec<_> = path.split('/').collect();
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(format!("{method} {path}\n{body}").as_bytes())
    );
    if path == "/system/agents" {
        if app.inner.factory.is_none() && app.inner.approval.is_some() {
            return Err(ApiError::conflict("system_read_only"));
        }
        if method != Method::PUT {
            return Err(ApiError::new(405, "method_not_allowed"));
        }
        let response = app
            .inner
            .store
            .lock()
            .map_err(|_| ApiError::internal())?
            .system_command(&key, &fingerprint, body, app.inner.factory.as_ref())?;
        app.changed();
        return Ok(Json(response).into_response());
    }
    if path == "/plans"
        || (parts.len() == 4 && parts[1] == "plans" && Uuid::parse_str(parts[2]).is_ok())
    {
        if method != Method::POST {
            return Err(ApiError::new(405, "method_not_allowed"));
        }
        let (id, action) = if path == "/plans" {
            (None, "generate")
        } else {
            (Some(parts[2]), parts[3])
        };
        let (status, response) = app
            .inner
            .store
            .lock()
            .map_err(|_| ApiError::internal())?
            .plan_command(
                app.inner.factory.as_ref(),
                &key,
                &fingerprint,
                id,
                action,
                body,
            )?;
        app.changed();
        return Ok((
            StatusCode::from_u16(status).map_err(|_| ApiError::internal())?,
            Json(response),
        )
            .into_response());
    }
    if path == "/projects" || (parts.len() == 3 && parts[1] == "projects") {
        let creating = path == "/projects";
        if (creating && method != Method::POST) || (!creating && method != Method::PATCH) {
            return Err(ApiError::new(405, "method_not_allowed"));
        }
        let replay = {
            let s = app.inner.store.lock().map_err(|_| ApiError::internal())?;
            s.db.query_row(
                "SELECT fingerprint,status,response FROM commands WHERE key=?1",
                [&key],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, u16>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(store::internal)?
        };
        if let Some((prior, status, response)) = replay {
            if prior != fingerprint {
                return Err(ApiError::conflict("idempotency_conflict"));
            }
            return Ok((
                StatusCode::from_u16(status).map_err(|_| ApiError::internal())?,
                Json(serde_json::from_str::<Value>(&response).map_err(store::internal)?),
            )
                .into_response());
        }
        let prepared = if creating {
            Some(
                app.inner
                    .factory
                    .as_ref()
                    .ok_or_else(|| ApiError::unavailable("project_setup_unavailable"))?
                    .prepare(&body)
                    .await?,
            )
        } else if body["reconnect"] == true {
            let project = {
                let store = app.inner.store.lock().map_err(|_| ApiError::internal())?;
                projects::get(&store.db, parts[2])?
            };
            if !project["lane_run_id"].is_null() {
                return Err(ApiError::conflict("project_needs_reconciliation"));
            }
            if project["runtime_status"] == "preparing" {
                return Err(ApiError::conflict("project_preparing"));
            }
            if project["local_path"]
                .as_str()
                .is_some_and(|p| p.starts_with("repository:"))
            {
                Some(project)
            } else {
                Some(
                    app.inner
                        .factory
                        .as_ref()
                        .ok_or_else(|| ApiError::unavailable("project_setup_unavailable"))?
                        .reconnect(&project)
                        .await?,
                )
            }
        } else {
            None
        };
        let (status, response) = app
            .inner
            .store
            .lock()
            .map_err(|_| ApiError::internal())?
            .project_command(
                &key,
                &fingerprint,
                if creating { None } else { Some(parts[2]) },
                body,
                prepared,
            )?;
        app.changed();
        return Ok((
            StatusCode::from_u16(status).map_err(|_| ApiError::internal())?,
            Json(response),
        )
            .into_response());
    }
    let execution_scope = if path == "/execution/pause" {
        Some(("pause", None))
    } else if parts.len() == 4 && Uuid::parse_str(parts[2]).is_ok() {
        match (parts[1], parts[3]) {
            ("tasks", "run") => Some(("run", Some(parts[2]))),
            ("runs", "cancel") => Some(("cancel", Some(parts[2]))),
            ("runs", "reconcile") => Some(("reconcile", Some(parts[2]))),
            _ => None,
        }
    } else {
        None
    };
    if let Some((scope, id)) = execution_scope {
        if method != Method::POST {
            return Err(ApiError::new(405, "method_not_allowed"));
        }
        let selected = if let Some(id) = id {
            let value = {
                let s = app.inner.store.lock().map_err(|_| ApiError::internal())?;
                if scope == "run" {
                    s.task(id)?
                } else {
                    s.current_run(id)?
                }
            };
            if scope == "run" {
                app.task_run_approval(&value).ok().flatten()
            } else {
                app.run_approval(&value).ok()
            }
        } else {
            app.inner.approval.clone()
        };
        let (status, response) = app
            .inner
            .store
            .lock()
            .map_err(|_| ApiError::internal())?
            .execution_command_guarded(
                &key,
                &fingerprint,
                scope,
                id,
                body,
                execution_store::Admission {
                    approval: selected.as_ref(),
                    ownership: app.inner.ownership.as_ref(),
                    automatic: None,
                    repository_queue: false,
                    coding_agents: None,
                    agent_registry: app.inner.factory.as_ref().map(|_| &registry),
                },
            )?;
        app.changed();
        return Ok((
            StatusCode::from_u16(status).map_err(|_| ApiError::internal())?,
            Json(response),
        )
            .into_response());
    }
    let (task_id, action) = if path == "/tasks" && method == Method::POST {
        (None, "create")
    } else if parts.len() >= 3 && parts[1] == "tasks" && Uuid::parse_str(parts[2]).is_ok() {
        if parts.len() == 3 && method == Method::PATCH {
            (Some(parts[2]), "edit")
        } else if parts.len() == 4
            && method == Method::POST
            && ["accept", "archive", "restore", "reopen", "block"].contains(&parts[3])
        {
            (Some(parts[2]), parts[3])
        } else {
            return Err(ApiError::new(405, "method_not_allowed"));
        }
    } else {
        return Err(ApiError::new(404, "not_found"));
    };
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(format!("{method} {path}\n{body}").as_bytes())
    );
    let runtime_task = if body["project_id"].is_string() {
        body.clone()
    } else if let Some(id) = task_id {
        app.inner
            .store
            .lock()
            .map_err(|_| ApiError::internal())?
            .task(id)?
    } else {
        body.clone()
    };
    let selected = app.task_approval(&runtime_task).ok().flatten();
    let (status, response) = app
        .inner
        .store
        .lock()
        .map_err(|_| ApiError::internal())?
        .command_guarded(
            &key,
            &fingerprint,
            task_id,
            action,
            body,
            execution_store::Admission {
                approval: selected.as_ref(),
                ownership: app.inner.ownership.as_ref(),
                automatic: app.inner.automation.as_ref(),
                repository_queue: app.inner.factory.is_some(),
                coding_agents: app.inner.factory.as_ref().and_then(|f| f.native_agents()),
                agent_registry: app.inner.factory.as_ref().map(|_| &registry),
            },
        )?;
    app.changed();
    Ok((
        StatusCode::from_u16(status).map_err(|_| ApiError::internal())?,
        Json(response),
    )
        .into_response())
}
