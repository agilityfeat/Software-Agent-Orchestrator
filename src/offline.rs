//! Quiescent, same-schema SQLite copies. This does not rewind external action journals.
use crate::{ApiError, Result, storage_ownership, store::internal};
use rusqlite::{
    Connection, OpenFlags,
    backup::{Backup, StepResult},
};
use std::{
    fs::File,
    path::{Path, PathBuf},
};

fn private_path(path: &Path, exists: bool) -> Result<()> {
    storage_ownership::validate(path, exists)?;
    let parent = path
        .parent()
        .ok_or_else(|| ApiError::bad("database_path_invalid"))?;
    if parent.canonicalize().map_err(internal)? != parent {
        return Err(ApiError::bad("database_path_invalid"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if parent.metadata().map_err(internal)?.permissions().mode() & 0o077 != 0 {
            return Err(ApiError::bad("database_permissions_invalid"));
        }
    }
    // SQLite can follow these sidecars even when the main DB is a safe regular file.
    for suffix in ["-wal", "-shm", "-journal"] {
        let side = PathBuf::from(format!("{}{suffix}", path.display()));
        match std::fs::symlink_metadata(&side) {
            Ok(_) if !exists => return Err(ApiError::conflict("database_destination_exists")),
            Ok(_) => storage_ownership::validate(&side, true)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(ApiError::bad("database_path_invalid")),
        }
    }
    Ok(())
}
fn verify(db: &Connection) -> Result<()> {
    let app: i64 = db
        .query_row("PRAGMA application_id", [], |r| r.get(0))
        .map_err(internal)?;
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(internal)?;
    if app != 0x57425253 || version != 2 {
        return Err(ApiError::bad("database_not_workboard_rs"));
    }
    let integrity: String = db
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(internal)?;
    if integrity != "ok" {
        return Err(ApiError::bad("database_integrity_failed"));
    }
    let (paused, lane): (i64, Option<String>) = db
        .query_row(
            "SELECT paused,lane_run_id FROM execution WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(internal)?;
    let active: i64 = db.query_row("SELECT count(*) FROM runs WHERE json_extract(document,'$.status') NOT IN ('succeeded','failed','cancelled') OR json_extract(document,'$.status') IS NULL", [], |r| r.get(0)).map_err(internal)?;
    let pending: i64 = db
        .query_row(
            "SELECT count(*) FROM attempts WHERE json_extract(document,'$.state') ='attempted'",
            [],
            |r| r.get(0),
        )
        .map_err(internal)?;
    let has_projects: i64 = db
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='project_lanes'",
            [],
            |r| r.get(0),
        )
        .map_err(internal)?;
    let project_lanes = if has_projects > 0 {
        db.query_row("SELECT count(*) FROM project_lanes", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(internal)?
    } else {
        0
    };
    let has_plans: i64 = db
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='plans'",
            [],
            |r| r.get(0),
        )
        .map_err(internal)?;
    let active_plans = if has_plans > 0 {
        db.query_row("SELECT count(*) FROM plans WHERE json_extract(document,'$.status') IN ('queued','generating','unknown','cleanup_pending')",[],|r|r.get::<_,i64>(0)).map_err(internal)?
    } else {
        0
    };
    if active_plans > 0 {
        return Err(ApiError::conflict("database_not_quiescent"));
    }
    if project_lanes > 0 {
        return Err(ApiError::conflict("database_not_quiescent"));
    }
    if paused != 1 || lane.is_some() || active != 0 || pending != 0 {
        return Err(ApiError::conflict("database_not_quiescent"));
    }
    Ok(())
}
/// Both `snapshot` and `restore` copy a paused, terminal database into a fresh file.
/// The source is never migrated or recovered; no execution configuration is read.
pub fn copy(source: &Path, destination: &Path) -> Result<()> {
    private_path(source, true)?;
    private_path(destination, false)?;
    if std::fs::symlink_metadata(destination).is_ok() {
        return Err(ApiError::conflict("database_destination_exists"));
    }
    let _owner = storage_ownership::lock(source)?;
    let db = Connection::open_with_flags(
        source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(internal)?;
    db.busy_timeout(std::time::Duration::from_millis(250))
        .map_err(internal)?;
    db.execute_batch("BEGIN;").map_err(internal)?;
    verify(&db)?;
    let temporary = destination
        .parent()
        .unwrap()
        .join(format!(".workboard-copy-{}.sqlite", uuid::Uuid::new_v4()));
    let result = (|| {
        let file = storage_ownership::private_options()
            .create_new(true)
            .open(&temporary)
            .map_err(internal)?;
        drop(file);
        let mut target = Connection::open(&temporary).map_err(internal)?;
        {
            let backup = Backup::new(&db, &mut target).map_err(internal)?;
            if backup.step(-1).map_err(internal)? != StepResult::Done {
                return Err(ApiError::conflict("database_copy_busy"));
            }
        }
        target
            .execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;")
            .map_err(internal)?;
        verify(&target)?;
        target.close().map_err(|(_, e)| internal(e))?;
        File::open(&temporary)
            .map_err(internal)?
            .sync_all()
            .map_err(internal)?;
        // hard_link provides atomic no-replace publication. Never rename over a DB.
        std::fs::hard_link(&temporary, destination)
            .map_err(|_| ApiError::conflict("database_destination_exists"))?;
        std::fs::remove_file(&temporary).map_err(internal)?;
        File::open(destination.parent().unwrap())
            .map_err(internal)?
            .sync_all()
            .map_err(internal)?;
        Ok(())
    })();
    // Only our unique temporary names are removed; source and destination are untouched.
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", temporary.display()));
    }
    result
}
