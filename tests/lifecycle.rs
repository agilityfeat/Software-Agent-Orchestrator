//! Synthetic lifecycle/storage evidence only: no provider, Docker, or GitHub calls.
#![cfg(unix)]
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use workboard_rs::{App, offline};
static PROCESS_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
const KEY: &str = "synthetic-lifecycle-key-0123456789abcdef";
fn root() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}
fn seed(path: &Path) {
    drop(App::open(path, KEY, 4311).unwrap());
    let db = Connection::open(path).unwrap();
    db.execute(
        "INSERT INTO tasks VALUES ('task',?1)",
        [
            json!({"id":"task","title":"Persistent synthetic task","status":"ready","revision":4})
                .to_string(),
        ],
    )
    .unwrap();
    db.execute(
        "INSERT INTO events(task_id,document) VALUES ('task',?1)",
        [json!({"type":"task.updated","task_id":"task","revision":4}).to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO commands VALUES ('original-key','fingerprint',200,?1)",
        [json!({"task":{"id":"task","revision":4}}).to_string()],
    )
    .unwrap();
    db.execute("INSERT INTO runs VALUES ('run','task',?1)",[json!({"id":"run","task_id":"task","status":"failed","stage":"observe","reason":"reconciled_without_delivery"}).to_string()]).unwrap();
    db.execute(
        "INSERT INTO attempts VALUES ('old-action','run',?1)",
        [
            json!({"id":"old-action","run_id":"run","state":"unknown","action":"coder"})
                .to_string(),
        ],
    )
    .unwrap();
}
fn records(path: &Path) -> Vec<String> {
    let db = Connection::open(path).unwrap();
    [
        "tasks",
        "events",
        "commands",
        "runs",
        "attempts",
        "execution",
    ]
    .into_iter()
    .map(|table| {
        let mut q = db
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let count = q.column_count();
        q.query_map([], |row| {
            Ok((0..count)
                .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                .collect::<Vec<_>>()
                .join("|"))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect::<Vec<_>>()
        .join("\n")
    })
    .collect()
}
#[test]
fn offline_snapshot_and_restore_preserve_history_and_refuse_live_unsafe_or_uncertain_state() {
    let _serial = PROCESS_TEST.lock().unwrap();
    let root = root();
    let dir = root.path().canonicalize().unwrap();
    let source = dir.join("source.sqlite");
    seed(&source);
    let before = records(&source);
    let snapshot = dir.join("snapshot.sqlite");
    let live = App::open(&source, KEY, 4311).unwrap();
    assert!(
        offline::copy(&source, &snapshot)
            .unwrap_err()
            .to_string()
            .contains("database_already_owned")
    );
    assert!(!snapshot.exists());
    drop(live);
    offline::copy(&source, &snapshot).unwrap();
    let restored = dir.join("restored.sqlite");
    let output = Command::new(env!("CARGO_BIN_EXE_workboard-rs"))
        .env_clear()
        .args(["restore"])
        .arg(&snapshot)
        .arg(&restored)
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(before, records(&snapshot));
    assert_eq!(before, records(&restored));
    assert_eq!(before, records(&source));
    assert_eq!(
        std::fs::metadata(&restored).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(offline::copy(&source, &restored).is_err());
    for suffix in ["-wal", "-shm", "-journal"] {
        let target = dir.join("stale-sidecar.sqlite");
        let sidecar = PathBuf::from(format!("{}{suffix}", target.display()));
        std::fs::write(&sidecar, b"unrelated existing journal").unwrap();
        std::fs::set_permissions(&sidecar, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(offline::copy(&source, &target).is_err());
        assert!(!target.exists());
        assert_eq!(
            std::fs::read(&sidecar).unwrap(),
            b"unrelated existing journal"
        );
        std::fs::remove_file(sidecar).unwrap();
    }
    let alias = dir.join("alias.sqlite");
    std::fs::hard_link(&source, &alias).unwrap();
    assert!(App::open(&alias, KEY, 4312).is_err());
    assert!(offline::copy(&alias, &dir.join("hardlink.sqlite")).is_err());
    std::fs::remove_file(alias).unwrap();
    let link = dir.join("symlink.sqlite");
    std::os::unix::fs::symlink(&source, &link).unwrap();
    assert!(offline::copy(&link, &dir.join("linked.sqlite")).is_err());
    let db = Connection::open(&source).unwrap();
    for (sql, restore) in [
        (
            "UPDATE execution SET paused=0",
            "UPDATE execution SET paused=1",
        ),
        (
            "UPDATE execution SET lane_run_id='run'",
            "UPDATE execution SET lane_run_id=NULL",
        ),
        (
            "UPDATE runs SET document=json_set(document,'$.status','unknown')",
            "UPDATE runs SET document=json_set(document,'$.status','failed')",
        ),
        (
            "UPDATE attempts SET document=json_set(document,'$.state','attempted')",
            "UPDATE attempts SET document=json_set(document,'$.state','unknown')",
        ),
    ] {
        db.execute_batch(sql).unwrap();
        assert!(offline::copy(&source, &dir.join("refused.sqlite")).is_err());
        assert!(!dir.join("refused.sqlite").exists());
        db.execute_batch(restore).unwrap();
    }
    db.execute(
        "INSERT INTO plans VALUES('pending-plan',?1)",
        [json!({"id":"pending-plan","status":"unknown"}).to_string()],
    )
    .unwrap();
    assert!(offline::copy(&source, &dir.join("pending-plan.sqlite")).is_err());
    db.execute("DELETE FROM plans WHERE id='pending-plan'", [])
        .unwrap();
    let unsafe_dir = dir.join("public");
    std::fs::create_dir(&unsafe_dir).unwrap();
    std::fs::set_permissions(&unsafe_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(offline::copy(&source, &unsafe_dir.join("copy.sqlite")).is_err());
    let foreign = dir.join("foreign.sqlite");
    Connection::open(&foreign)
        .unwrap()
        .execute_batch("PRAGMA application_id=123;")
        .unwrap();
    std::fs::set_permissions(&foreign, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(offline::copy(&foreign, &dir.join("foreign-copy.sqlite")).is_err());
    assert_eq!(before, records(&source));
}
#[test]
fn snapshot_includes_committed_wal_without_changing_source_journal() {
    let _serial = PROCESS_TEST.lock().unwrap();
    let root = root();
    let dir = root.path().canonicalize().unwrap();
    let source = dir.join("wal.sqlite");
    seed(&source);
    let db = Connection::open(&source).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
        .unwrap();
    db.execute(
        "INSERT INTO commands VALUES ('wal-key','fingerprint',200,'{}')",
        [],
    )
    .unwrap();
    let wal = PathBuf::from(format!("{}-wal", source.display()));
    let before = std::fs::read(&wal).unwrap();
    let target = dir.join("copy.sqlite");
    offline::copy(&source, &target).unwrap();
    assert_eq!(records(&source), records(&target));
    assert_eq!(before, std::fs::read(wal).unwrap());
}
struct Service(Child);
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
fn start(db: &Path, port: u16) -> Service {
    Service(
        Command::new(env!("CARGO_BIN_EXE_workboard-rs"))
            .env_clear()
            .env("WORKBOARD_DB", db)
            .env("WORKBOARD_OPERATOR_KEY", KEY)
            .env("WORKBOARD_PORT", port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}
fn wait_exit(child: &mut Child) -> std::process::ExitStatus {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(
            Instant::now() < until,
            "service did not exit within five seconds"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn request(port: u16, method: &str, path: &str, body: &str, cookie: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(stream,"{method} /api/workboard/v1{path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).unwrap();
    out
}
#[test]
fn sigterm_closes_sse_releases_lock_and_restart_preserves_pause_and_nonreplay() {
    let _serial = PROCESS_TEST.lock().unwrap();
    let root = root();
    let dir = root.path().canonicalize().unwrap();
    let path = dir.join("service.sqlite");
    seed(&path);
    let db = Connection::open(&path).unwrap();
    db.execute(
        "UPDATE runs SET document=json_set(document,'$.status','running','$.stage','coder')",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE attempts SET document=json_set(document,'$.state','attempted')",
        [],
    )
    .unwrap();
    db.execute("UPDATE execution SET lane_run_id='run'", [])
        .unwrap();
    drop(db);
    let port = port();
    let mut service = start(&path, port);
    let until = Instant::now() + Duration::from_secs(5);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut duplicate = start(&path, super_port(port));
    assert!(!wait_exit(&mut duplicate.0).success());
    let response = request(
        port,
        "POST",
        "/session",
        &json!({"key":KEY}).to_string(),
        "",
    );
    assert!(response.starts_with("HTTP/1.1 200"));
    let cookie = response
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("set-cookie:"))
        .unwrap()
        .split_once(':')
        .unwrap()
        .1
        .trim()
        .split(';')
        .next()
        .unwrap();
    let snapshot = request(port, "GET", "/snapshot", "", cookie);
    let value: Value = serde_json::from_str(snapshot.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(value["execution"]["paused"], true);
    assert_eq!(value["runs"][0]["status"], "unknown");
    let mut sse = TcpStream::connect(("127.0.0.1", port)).unwrap();
    sse.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(sse,"GET /api/workboard/v1/events HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: {cookie}\r\n\r\n").unwrap();
    let mut initial = [0; 1024];
    let count = sse.read(&mut initial).unwrap();
    assert!(String::from_utf8_lossy(&initial[..count]).contains("200 OK"));
    unsafe {
        assert_eq!(libc::kill(service.0.id() as i32, libc::SIGTERM), 0);
    }
    assert!(wait_exit(&mut service.0).success());
    let mut rest = Vec::new();
    sse.read_to_end(&mut rest).unwrap();
    assert!(!String::from_utf8_lossy(&rest).contains("session_expired"));
    let before = records(&path);
    let mut restarted = start(&path, port);
    let until = Instant::now() + Duration::from_secs(5);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(request(port, "GET", "/snapshot", "", cookie).starts_with("HTTP/1.1 200"));
    unsafe {
        libc::kill(restarted.0.id() as i32, libc::SIGTERM);
    }
    assert!(wait_exit(&mut restarted.0).success());
    let after = records(&path);
    for index in [0, 2, 4, 5] {
        assert_eq!(before[index], after[index]);
    }
    let db = Connection::open(&path).unwrap();
    let status: String = db
        .query_row(
            "SELECT json_extract(document,'$.status') FROM runs",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "unknown");
    drop(db);
    drop(App::open(&path, KEY, port).unwrap());
}
fn super_port(other: u16) -> u16 {
    loop {
        let value = port();
        if value != other {
            return value;
        }
    }
}
