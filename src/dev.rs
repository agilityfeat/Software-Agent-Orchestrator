//! One-command development startup using private credential references only.
use crate::{ApiError, Result, store::internal};
use serde::Deserialize;
use std::path::{Path, PathBuf};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Launch {
    schema_version: u32,
    database_path: PathBuf,
    port: u16,
    operator_key_file: PathBuf,
    setup_file: PathBuf,
    setup_sha256: String,
    setup_key_file: PathBuf,
    provider_home: PathBuf,
    github_token_env: String,
}
fn secret(path: &Path) -> Result<String> {
    crate::execution_config::private_file(path)?;
    String::from_utf8(std::fs::read(path).map_err(internal)?)
        .map(|s| s.trim_end_matches(['\r', '\n']).to_string())
        .map_err(internal)
}
pub async fn launch(path: &Path) -> Result<()> {
    crate::execution_config::private_file(path)?;
    let c: Launch = serde_json::from_slice(&std::fs::read(path).map_err(internal)?)
        .map_err(|_| ApiError::bad("dev_configuration_invalid"))?;
    if c.schema_version != 1
        || !c.database_path.is_absolute()
        || !["GH_TOKEN", "GITHUB_TOKEN"].contains(&c.github_token_env.as_str())
    {
        return Err(ApiError::bad("dev_configuration_invalid"));
    }
    let setup_key = secret(&c.setup_key_file)?;
    let token = match std::env::var(&c.github_token_env) {
        Ok(token) => token,
        Err(_) => {
            crate::projects::Factory::load(&c.setup_file, setup_key.as_bytes(), &c.setup_sha256)?
                .github_session_token()
                .await?
        }
    };
    let mut child = std::process::Command::new(std::env::current_exe().map_err(internal)?);
    child
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .env("WORKBOARD_DB", c.database_path)
        .env("WORKBOARD_PORT", c.port.to_string())
        .env("WORKBOARD_OPERATOR_KEY", secret(&c.operator_key_file)?)
        .env("WORKBOARD_PROJECT_SETUP", c.setup_file)
        .env("WORKBOARD_PROJECT_SETUP_SHA256", c.setup_sha256)
        .env("WORKBOARD_PROJECT_SETUP_KEY", setup_key)
        .env("CODEX_HOME", c.provider_home)
        .env("GH_TOKEN", token);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let _ = child.exec();
        Err(ApiError::unavailable("dev_start_failed"))
    }
    #[cfg(not(unix))]
    {
        let status = child.status().map_err(internal)?;
        if !status.success() {
            Err(ApiError::unavailable("dev_start_failed"))
        }
        Ok(())
    }
}
