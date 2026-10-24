//! Opt-in offline handover. Lifetime SQLite readers prevent CAS transfer while
//! either supported executor is alive; no TTL or PID-based takeover exists.
use crate::{ApiError, Result, storage_ownership, store::internal};
use rusqlite::{Connection, OpenFlags, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub schema_version: u32,
    pub database_path: PathBuf,
    pub record_id: String,
    pub repository: String,
    pub epoch: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Init {
    schema_version: u32,
    repository: String,
    node_board: PathBuf,
    node_state: PathBuf,
    rust_board: PathBuf,
    node_execution: PathBuf,
    node_guard: PathBuf,
    owner: String,
}
fn bad() -> ApiError {
    ApiError::conflict("execution_ownership_unavailable")
}
pub(crate) fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn private(path: &Path, existing: bool) -> Result<()> {
    storage_ownership::validate(path, existing)?;
    let parent = path.parent().ok_or_else(bad)?;
    if parent.canonicalize().map_err(internal)? != parent {
        return Err(bad());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if parent.metadata().map_err(internal)?.permissions().mode() & 0o077 != 0 {
            return Err(bad());
        }
    }
    Ok(())
}
fn identity(path: &Path) -> Result<Value> {
    private(path, true)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = path.metadata().map_err(internal)?;
        Ok(json!({"dev":m.dev().to_string(),"ino":m.ino().to_string()}))
    }
    #[cfg(not(unix))]
    {
        Err(bad())
    }
}
fn source_hash(path: &Path) -> Result<String> {
    if !path.is_absolute()
        || !path.symlink_metadata().is_ok_and(|m| m.is_file())
        || path.canonicalize().ok().as_deref() != Some(path)
    {
        return Err(bad());
    }
    let bytes = std::fs::read(path).map_err(internal)?;
    if bytes.len() > 16777216 {
        return Err(bad());
    }
    Ok(sha(&bytes))
}
fn db(path: &Path, read_only: bool) -> Result<Connection> {
    private(path, true)?;
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    let db = Connection::open_with_flags(path, flags).map_err(internal)?;
    db.busy_timeout(Duration::from_millis(200))
        .map_err(internal)?;
    Ok(db)
}
fn record(db: &Connection) -> Result<Value> {
    let (app, version): (i64, i64) = (
        db.query_row("PRAGMA application_id", [], |r| r.get(0))
            .map_err(internal)?,
        db.query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(internal)?,
    );
    let mode: String = db
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .map_err(internal)?;
    if app != 0x57424f57 || version != 1 || mode != "delete" {
        return Err(bad());
    }
    let raw: String = db
        .query_row(
            "SELECT document FROM authority WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(internal)?;
    let value: Value = serde_json::from_str(&raw).map_err(internal)?;
    let path: String = db
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |r| r.get(0),
        )
        .map_err(internal)?;
    if value["owner_identity"] != identity(Path::new(&path))? {
        return Err(bad());
    }
    if value["schema_version"] != 1
        || !value["record_id"]
            .as_str()
            .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok())
        || !value["epoch"].as_u64().is_some_and(|e| e > 0)
        || !["node", "rust"].contains(&value["owner"].as_str().unwrap_or(""))
        || !value["repository"]
            .as_str()
            .is_some_and(crate::store::valid_repository)
    {
        return Err(bad());
    }
    Ok(value)
}
fn check_identities(record: &Value) -> Result<()> {
    for name in ["node_board", "node_state", "rust_board"] {
        if identity(Path::new(record[name]["path"].as_str().ok_or_else(bad)?))?
            != record[name]["identity"]
        {
            return Err(bad());
        }
    }
    for name in ["node_execution", "node_guard", "rust_executable"] {
        if source_hash(Path::new(record[name]["path"].as_str().ok_or_else(bad)?))?
            != record[name]["sha256"].as_str().ok_or_else(bad)?
        {
            return Err(bad());
        }
    }
    Ok(())
}
/// SQL quote() values avoid number/JSON canonicalization differences across the
/// two implementations. Only the digest, never DB content, enters stop proofs.
pub(crate) fn fingerprint(db: &Connection) -> Result<String> {
    let mut statement = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .map_err(internal)?;
    let names = statement
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(internal)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(internal)?;
    let mut tables = Vec::new();
    for name in names {
        if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Err(bad());
        }
        let mut info = db
            .prepare(&format!("PRAGMA table_info(\"{name}\")"))
            .map_err(internal)?;
        let columns = info
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(internal)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(internal)?;
        if columns
            .iter()
            .any(|n| !n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        {
            return Err(bad());
        }
        let fields = columns
            .iter()
            .map(|n| format!("quote(\"{n}\")"))
            .collect::<Vec<_>>()
            .join(",");
        let mut query = db
            .prepare(&format!(
                "SELECT {fields} FROM \"{name}\" ORDER BY rowid LIMIT 100001"
            ))
            .map_err(internal)?;
        let rows = query
            .query_map([], |r| {
                (0..columns.len())
                    .map(|i| r.get::<_, String>(i))
                    .collect::<std::result::Result<Vec<_>, _>>()
            })
            .map_err(internal)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(internal)?;
        if rows.len() > 100000 {
            return Err(bad());
        }
        tables.push(json!([name, columns, rows]));
    }
    let bytes = serde_json::to_vec(&tables).map_err(internal)?;
    if bytes.len() > 134217728 {
        return Err(bad());
    }
    Ok(sha(&bytes))
}
fn zero(db: &Connection, sql: &str) -> Result<()> {
    if db
        .query_row(sql, [], |r| r.get::<_, i64>(0))
        .map_err(internal)?
        != 0
    {
        return Err(ApiError::conflict("ownership_store_not_quiescent"));
    }
    Ok(())
}
pub(crate) fn quiescent(db: &Connection, kind: &str) -> Result<()> {
    match kind {
        "rust_board" => {
            let app: i64 = db
                .query_row("PRAGMA application_id", [], |r| r.get(0))
                .map_err(internal)?;
            let version: i64 = db
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .map_err(internal)?;
            if app != 0x57425253 || version != 2 {
                return Err(bad());
            }
            zero(
                db,
                "SELECT count(*) FROM execution WHERE paused<>1 OR lane_run_id IS NOT NULL",
            )?;
            zero(
                db,
                "SELECT count(*) FROM runs WHERE json_extract(document,'$.status') NOT IN ('succeeded','failed','cancelled') OR json_extract(document,'$.status') IS NULL",
            )?;
            zero(
                db,
                "SELECT count(*) FROM attempts WHERE json_extract(document,'$.state')='attempted'",
            )?;
        }
        "node_board" => {
            zero(
                db,
                "SELECT count(*) FROM runs WHERE status NOT IN ('succeeded','failed','cancelled')",
            )?;
            zero(db, "SELECT count(*) FROM outbox WHERE status='pending'")?;
        }
        "node_state" => {
            let version: i64 = db
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .map_err(internal)?;
            if version != 4 {
                return Err(bad());
            }
            zero(
                db,
                "SELECT count(*) FROM claims WHERE claim_state IN ('claimed','uncertain','dispatched','quarantined','invalid')",
            )?;
            zero(
                db,
                "SELECT count(*) FROM execution_ownership WHERE released=0",
            )?;
            zero(db, "SELECT count(*) FROM controller_leases")?;
            zero(
                db,
                "SELECT count(*) FROM controller_safety_latch WHERE active=1",
            )?;
            zero(
                db,
                "SELECT count(*) FROM canary_sessions WHERE stage NOT IN ('complete','terminal')",
            )?;
        }
        _ => return Err(bad()),
    }
    Ok(())
}
fn proofs_path(path: &Path, kind: &str) -> PathBuf {
    PathBuf::from(format!("{}.{}-stop.json", path.display(), kind))
}
fn write_proof(path: &Path, value: &Value) -> Result<()> {
    match path.symlink_metadata() {
        Ok(_) => private(path, true)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(bad()),
    }
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut file = storage_ownership::private_options()
        .create_new(true)
        .open(&temp)
        .map_err(internal)?;
    file.write_all(&serde_json::to_vec(value).map_err(internal)?)
        .map_err(internal)?;
    file.sync_all().map_err(internal)?;
    std::fs::rename(&temp, path).map_err(internal)?;
    File::open(path.parent().unwrap())
        .map_err(internal)?
        .sync_all()
        .map_err(internal)
}
fn snapshots(record: &Value) -> Result<(Vec<Connection>, Value)> {
    let mut held = Vec::new();
    let mut hashes = serde_json::Map::new();
    for name in ["node_board", "node_state", "rust_board"] {
        let source = db(
            Path::new(record[name]["path"].as_str().ok_or_else(bad)?),
            true,
        )?;
        source.execute_batch("BEGIN;").map_err(internal)?;
        quiescent(&source, name)?;
        hashes.insert(name.into(), json!(fingerprint(&source)?));
        held.push(source);
    }
    Ok((held, Value::Object(hashes)))
}
pub struct Guard {
    connection: Connection,
    binding: Binding,
    nonce: String,
    identity: Value,
}
impl Guard {
    pub fn open(binding: &Binding, board: &Path) -> Result<Self> {
        if binding.schema_version != 1
            || binding.epoch == 0
            || uuid::Uuid::parse_str(&binding.record_id).is_err()
        {
            return Err(bad());
        }
        let connection = db(&binding.database_path, false)?;
        let nonce = uuid::Uuid::new_v4().to_string();
        let identity = identity(&binding.database_path)?;
        connection
            .execute_batch("BEGIN IMMEDIATE;")
            .map_err(|_| bad())?;
        let mut value = record(&connection)?;
        if value["record_id"] != binding.record_id
            || value["repository"] != binding.repository
            || value["epoch"] != binding.epoch
            || value["owner"] != "rust"
            || value["rust_board"]["path"] != board.to_string_lossy().as_ref()
            || value["rust_executable"]["path"]
                != std::env::current_exe()
                    .map_err(internal)?
                    .to_string_lossy()
                    .as_ref()
        {
            return Err(bad());
        }
        check_identities(&value)?;
        value["rust_session"] = json!(nonce);
        value["rust_session_epoch"] = json!(binding.epoch);
        connection
            .execute(
                "UPDATE authority SET document=?1 WHERE singleton=1",
                [value.to_string()],
            )
            .map_err(internal)?;
        connection
            .execute_batch("COMMIT; BEGIN;")
            .map_err(|_| bad())?;
        let guard = Self {
            connection,
            binding: binding.clone(),
            nonce,
            identity,
        };
        guard.assert(&binding.repository)?;
        Ok(guard)
    }
    pub fn assert(&self, repository: &str) -> Result<()> {
        if repository != self.binding.repository
            || identity(&self.binding.database_path)? != self.identity
        {
            return Err(bad());
        }
        let value = record(&self.connection)?; // SELECT acquires and retains SHARED lock.
        if value["owner"] != "rust"
            || value["record_id"] != self.binding.record_id
            || value["epoch"] != self.binding.epoch
            || value["rust_session"] != self.nonce
        {
            return Err(bad());
        }
        check_identities(&value)
    }
    pub fn finish(&self, board: &Connection) -> Result<()> {
        self.assert(&self.binding.repository)?;
        quiescent(board, "rust_board")?;
        let proof = json!({"schema_version":1,"record_id":self.binding.record_id,"repository":self.binding.repository,"epoch":self.binding.epoch,"session":self.nonce,"runtime":"rust","paused":true,"fingerprints":{"rust_board":fingerprint(board)?}});
        write_proof(&proofs_path(&self.binding.database_path, "rust"), &proof)
    }
}
pub fn cli(args: &[std::ffi::OsString]) -> Result<Value> {
    match args.first().and_then(|s| s.to_str()) {
        Some("init") if args.len() == 3 => {
            let input = Path::new(&args[1]);
            private(input, true)?;
            let bytes = std::fs::read(input).map_err(internal)?;
            if bytes.len() > 16384 {
                return Err(bad());
            }
            let init: Init = serde_json::from_slice(&bytes).map_err(internal)?;
            let output = Path::new(&args[2]);
            private(output, false)?;
            if init.schema_version != 1
                || !crate::store::valid_repository(&init.repository)
                || !["node", "rust"].contains(&init.owner.as_str())
            {
                return Err(bad());
            }
            let mut value = json!({"schema_version":1,"record_id":uuid::Uuid::new_v4().to_string(),"repository":init.repository,"owner":init.owner,"epoch":1,"node_session":null,"rust_session":null,"node_session_epoch":null,"rust_session_epoch":null});
            for (name, path) in [
                ("node_board", init.node_board),
                ("node_state", init.node_state),
                ("rust_board", init.rust_board),
            ] {
                value[name] = json!({"identity":identity(&path)?,"path":path});
            }
            for (name, path) in [
                ("node_execution", init.node_execution),
                ("node_guard", init.node_guard),
                (
                    "rust_executable",
                    std::env::current_exe().map_err(internal)?,
                ),
            ] {
                value[name] = json!({"sha256":source_hash(&path)?,"path":path});
            }
            let (_held, _) = snapshots(&value)?;
            for suffix in ["-wal", "-shm", "-journal"] {
                if std::fs::symlink_metadata(format!("{}{suffix}", output.display())).is_ok() {
                    return Err(bad());
                }
            }
            storage_ownership::private_options()
                .create_new(true)
                .open(output)
                .map_err(internal)?;
            value["owner_identity"] = identity(output)?;
            let connection = Connection::open(output).map_err(internal)?;
            connection.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA application_id=0x57424f57; PRAGMA user_version=1; CREATE TABLE authority(singleton INTEGER PRIMARY KEY CHECK(singleton=1),document TEXT NOT NULL);").map_err(internal)?;
            connection
                .execute("INSERT INTO authority VALUES(1,?1)", [value.to_string()])
                .map_err(internal)?;
            Ok(value)
        }
        Some("status") if args.len() == 2 => record(&db(Path::new(&args[1]), true)?),
        Some("transfer") if args.len() == 4 => {
            let path = Path::new(&args[1]);
            let expected = args[2]
                .to_str()
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or_else(bad)?;
            let target = args[3]
                .to_str()
                .filter(|s| ["node", "rust"].contains(s))
                .ok_or_else(bad)?;
            let _rust_owner;
            let connection = db(path, false)?;
            connection
                .execute_batch("BEGIN IMMEDIATE;")
                .map_err(|_| bad())?;
            let mut value = record(&connection)?;
            if value["epoch"] != expected || value["owner"] == target {
                return Err(bad());
            }
            check_identities(&value)?;
            _rust_owner = storage_ownership::lock(Path::new(
                value["rust_board"]["path"].as_str().ok_or_else(bad)?,
            ))?;
            let (_held, hashes) = snapshots(&value)?;
            for kind in ["node", "rust"] {
                if value[format!("{kind}_session")].is_null() {
                    continue;
                }
                let proof_path = proofs_path(path, kind);
                private(&proof_path, true)?;
                let bytes = std::fs::read(proof_path).map_err(internal)?;
                if bytes.len() > 4096 {
                    return Err(bad());
                }
                let proof: Value = serde_json::from_slice(&bytes).map_err(internal)?;
                if proof["schema_version"] != 1
                    || proof["record_id"] != value["record_id"]
                    || proof["repository"] != value["repository"]
                    || proof["runtime"] != kind
                    || proof["paused"] != true
                    || proof["session"] != value[format!("{kind}_session")]
                    || proof["epoch"] != value[format!("{kind}_session_epoch")]
                {
                    return Err(ApiError::conflict("ownership_stop_proof_required"));
                }
                for name in if kind == "node" {
                    vec!["node_board", "node_state"]
                } else {
                    vec!["rust_board"]
                } {
                    if proof["fingerprints"][name] != hashes[name] {
                        return Err(ApiError::conflict("ownership_stop_proof_stale"));
                    }
                }
            }
            value["owner"] = json!(target);
            value["epoch"] = json!(
                expected
                    .checked_add(1)
                    .filter(|n| *n < 9007199254740991)
                    .ok_or_else(bad)?
            );
            if connection.execute("UPDATE authority SET document=?1 WHERE singleton=1 AND json_extract(document,'$.epoch')=?2",params![value.to_string(),expected]).map_err(internal)?!=1 {return Err(bad());}
            connection.execute_batch("COMMIT;").map_err(|_| bad())?;
            Ok(value)
        }
        _ => Err(ApiError::bad("ownership_command_invalid")),
    }
}
