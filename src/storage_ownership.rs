//! The service and offline copy commands share one advisory owner lock per database.
use crate::{ApiError, Result, store::internal};
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

pub(crate) fn validate(path: &Path, required: bool) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| c == std::path::Component::ParentDir)
    {
        return Err(ApiError::bad("database_path_invalid"));
    }
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.is_file() {
                return Err(ApiError::bad("database_path_invalid"));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::{MetadataExt, PermissionsExt};
                if meta.nlink() != 1 {
                    return Err(ApiError::bad("database_path_invalid"));
                }
                if meta.permissions().mode() & 0o077 != 0 {
                    return Err(ApiError::bad("database_permissions_invalid"));
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !required => {}
        Err(_) => return Err(ApiError::bad("database_path_invalid")),
    }
    let parent = path
        .parent()
        .ok_or_else(|| ApiError::bad("database_path_invalid"))?
        .canonicalize()
        .map_err(internal)?;
    if parent.to_string_lossy().contains("/.openclaw")
        || parent.to_string_lossy().contains("CodexWorkboardRuntime")
    {
        return Err(ApiError::bad("database_path_invalid"));
    }
    Ok(())
}
pub(crate) fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    options
}
pub(crate) fn lock(path: &Path) -> Result<File> {
    validate(path, false)?;
    let lock_path = path.with_extension("workboard-rs.lock");
    validate(&lock_path, false)?;
    let lock = private_options()
        .create(true)
        .open(&lock_path)
        .map_err(internal)?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .map_err(|_| ApiError::conflict("database_already_owned"))?;
    Ok(lock)
}
