//! Finite reviewed validation recipes. No task data selects an executable or argv.
use super::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "id", deny_unknown_fields)]
pub enum ValidationProfile {
    #[serde(rename = "python-unittest-v1")]
    Python {},
    #[serde(rename = "node-builtin-review-v1")]
    NodeBuiltin {
        manifest_sha256: String,
        build_sha256: String,
    },
    #[serde(rename = "node-unit-syntax-v1")]
    NodeUnit { manifest_sha256: String },
    #[serde(rename = "node-vite-v1")]
    NodeVite {
        manifest_sha256: String,
        lock_sha256: String,
        cache_sha256: String,
    },
    #[serde(rename = "node-npm-v1")]
    Node {
        manifest_sha256: String,
        lock_sha256: String,
        check_sha256: String,
        cache_sha256: String,
    },
}
impl ValidationProfile {
    pub(super) fn validate(&self) -> Result<()> {
        if let Self::NodeVite {
            manifest_sha256,
            lock_sha256,
            cache_sha256,
        } = self
        {
            if [manifest_sha256, lock_sha256, cache_sha256]
                .iter()
                .any(|s| !hex64(s))
            {
                return Err(err("adapter_profile_invalid"));
            }
        }
        if let Self::NodeUnit { manifest_sha256 } = self {
            if !hex64(manifest_sha256) {
                return Err(err("adapter_profile_invalid"));
            }
        }
        if let Self::NodeBuiltin {
            manifest_sha256,
            build_sha256,
        } = self
        {
            if !hex64(manifest_sha256) || !hex64(build_sha256) {
                return Err(err("adapter_profile_invalid"));
            }
        }
        if let Self::Node {
            manifest_sha256,
            lock_sha256,
            check_sha256,
            cache_sha256,
        } = self
        {
            if [manifest_sha256, lock_sha256, check_sha256, cache_sha256]
                .iter()
                .any(|s| !hex64(s))
            {
                return Err(err("adapter_profile_invalid"));
            }
        }
        Ok(())
    }
    pub(super) fn node(&self) -> bool {
        matches!(
            self,
            Self::Node { .. }
                | Self::NodeBuiltin { .. }
                | Self::NodeVite { .. }
                | Self::NodeUnit { .. }
        )
    }
}
pub(super) fn node(profile: &Option<ValidationProfile>) -> bool {
    profile.as_ref().is_some_and(ValidationProfile::node)
}
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read_file(root: &Path, name: &str, max: u64) -> Result<Vec<u8>> {
    let path = root.join(name);
    let meta = std::fs::symlink_metadata(&path).map_err(|_| err("adapter_profile_invalid"))?;
    if !meta.is_file()
        || meta.len() > max
        || path.canonicalize().ok().as_deref() != Some(path.as_path())
    {
        return Err(err("adapter_profile_invalid"));
    }
    std::fs::read(path).map_err(|_| err("adapter_profile_invalid"))
}
pub(super) fn project_pins(root: &Path, profile: &ValidationProfile) -> Result<()> {
    if let ValidationProfile::NodeUnit { manifest_sha256 } = profile {
        let raw = read_file(root, "package.json", 1048576)?;
        if sha(&raw) != *manifest_sha256 {
            return Err(err("adapter_profile_drift"));
        }
        validate_unit_manifest(&raw)?;
        for path in ["node_modules", "package-lock.json", "npm-shrinkwrap.json"] {
            if root.join(path).symlink_metadata().is_ok() {
                return Err(err("adapter_profile_invalid"));
            }
        }
        return Ok(());
    }
    if let ValidationProfile::NodeVite {
        manifest_sha256,
        lock_sha256,
        ..
    } = profile
    {
        let p = read_file(root, "package.json", 1048576)?;
        let l = read_file(root, "package-lock.json", 4194304)?;
        if sha(&p) != *manifest_sha256 || sha(&l) != *lock_sha256 {
            return Err(err("adapter_profile_drift"));
        }
        if root.join(".npmrc").symlink_metadata().is_ok() {
            return Err(err("adapter_profile_invalid"));
        }
        return validate_vite_inputs(&p, &l);
    }
    if let ValidationProfile::NodeBuiltin {
        manifest_sha256,
        build_sha256,
    } = profile
    {
        if std::fs::symlink_metadata(root.join("node_modules")).is_ok() {
            return Err(err("adapter_profile_invalid"));
        }
        for (name, want, max) in [
            ("package.json", manifest_sha256, 1048576),
            ("scripts/build.js", build_sha256, 65536),
        ] {
            if sha(&read_file(root, name, max)?) != *want {
                return Err(err("adapter_profile_drift"));
            }
        }
        let p: Value = serde_json::from_slice(&read_file(root, "package.json", 1048576)?)
            .map_err(|_| err("adapter_profile_invalid"))?;
        if p["scripts"]["test"] != "node --test test/*.test.js"
            || p["scripts"]["build"] != "node scripts/build.js"
        {
            return Err(err("adapter_profile_invalid"));
        }
        return Ok(());
    }
    let ValidationProfile::Node {
        manifest_sha256,
        lock_sha256,
        check_sha256,
        ..
    } = profile
    else {
        return Err(err("adapter_profile_invalid"));
    };
    for (name, want, max) in [
        ("package.json", manifest_sha256, 1048576),
        ("package-lock.json", lock_sha256, 4194304),
        ("scripts/check.js", check_sha256, 65536),
    ] {
        if sha(&read_file(root, name, max)?) != *want {
            return Err(err("adapter_profile_drift"));
        }
    }
    if std::fs::symlink_metadata(root.join(".npmrc")).is_ok() {
        return Err(err("adapter_profile_invalid"));
    }
    let package: Value = serde_json::from_slice(&read_file(root, "package.json", 1048576)?)
        .map_err(|_| err("adapter_profile_invalid"))?;
    if package["scripts"]["check"] != "node scripts/check.js"
        || package["scripts"]["test"] != "node --test"
    {
        return Err(err("adapter_profile_invalid"));
    }
    Ok(())
}
fn cache_pins(profile: &ValidationProfile) -> Result<()> {
    let (cache_sha256, manifest_sha256, lock_sha256, check_sha256, id) = match profile {
        ValidationProfile::Node {
            cache_sha256,
            manifest_sha256,
            lock_sha256,
            check_sha256,
        } => (
            cache_sha256,
            manifest_sha256,
            lock_sha256,
            Some(check_sha256),
            "node-npm-v1",
        ),
        ValidationProfile::NodeVite {
            cache_sha256,
            manifest_sha256,
            lock_sha256,
        } => (
            cache_sha256,
            manifest_sha256,
            lock_sha256,
            None,
            "node-vite-v1",
        ),
        _ => return Err(err("adapter_profile_invalid")),
    };
    let bytes = read_file(
        Path::new("/opt/workboard-node-profile"),
        "cache.json",
        65536,
    )?;
    if sha(&bytes) != *cache_sha256 {
        return Err(err("adapter_profile_drift"));
    }
    let marker: Value =
        serde_json::from_slice(&bytes).map_err(|_| err("adapter_profile_invalid"))?;
    if marker["schema_version"] != 1
        || marker["id"] != id
        || marker["manifest_sha256"] != *manifest_sha256
        || marker["lock_sha256"] != *lock_sha256
        || check_sha256.is_some_and(|v| marker["check_sha256"] != *v)
        || !marker["cache_files_sha256"].as_str().is_some_and(hex64)
    {
        return Err(err("adapter_profile_drift"));
    }
    Ok(())
}
#[derive(Clone)]
pub(super) struct Tree(BTreeMap<String, (String, bool)>);
impl Tree {
    pub(super) fn digest(&self) -> String {
        sha(serde_json::to_string(&self.0).unwrap().as_bytes())
    }
}
pub(super) fn source_tree(root: &Path) -> Result<Tree> {
    fn walk(
        root: &Path,
        path: &Path,
        files: &mut BTreeMap<String, (String, bool)>,
        size: &mut u64,
    ) -> Result<()> {
        for entry in std::fs::read_dir(path).map_err(|_| err("adapter_profile_invalid"))? {
            let entry = entry.map_err(|_| err("adapter_profile_invalid"))?;
            let relative = entry.path().strip_prefix(root).unwrap().to_path_buf();
            if path == root
                && [".git", "node_modules"].contains(&entry.file_name().to_string_lossy().as_ref())
            {
                continue;
            }
            let meta = entry
                .path()
                .symlink_metadata()
                .map_err(|_| err("adapter_profile_invalid"))?;
            if meta.is_dir() {
                walk(root, &entry.path(), files, size)?;
            } else if meta.is_file() {
                *size += meta.len();
                if *size > 67108864 || meta.len() > 4194304 || files.len() >= 10000 {
                    return Err(err("adapter_profile_invalid"));
                }
                #[cfg(unix)]
                let executable = {
                    use std::os::unix::fs::PermissionsExt;
                    meta.permissions().mode() & 0o111 != 0
                };
                #[cfg(not(unix))]
                let executable = false;
                let key = relative
                    .to_str()
                    .filter(|s| !s.contains(['\n', '\r']))
                    .ok_or_else(|| err("adapter_profile_invalid"))?
                    .to_string();
                files.insert(
                    key,
                    (
                        sha(&std::fs::read(entry.path())
                            .map_err(|_| err("adapter_profile_invalid"))?),
                        executable,
                    ),
                );
            } else {
                return Err(err("adapter_profile_invalid"));
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    walk(root, root, &mut files, &mut 0)?;
    Ok(Tree(files))
}
pub(super) async fn install(profile: &ValidationProfile) -> Result<()> {
    project_pins(Path::new("/workspace"), profile)?;
    if matches!(profile, ValidationProfile::NodeUnit { .. }) {
        unit_files(Path::new("/workspace"))?;
        return Ok(());
    }
    if matches!(profile, ValidationProfile::NodeBuiltin { .. }) {
        let tracked = super::contained::local(
            "/usr/bin/git",
            args(&["ls-files", "--", "dist"]),
            None,
            65536,
        )
        .await?;
        if !tracked.is_empty() {
            return Err(err("adapter_profile_invalid"));
        }
        return Ok(());
    }
    cache_pins(profile)?;
    let (status, _, _) = super::contained::local_result(
        "/usr/local/bin/npm",
        args(&[
            "ci",
            "--offline",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
            "--cache",
            "/opt/workboard-node-cache",
            "--logs-dir",
            "/tmp/npm-logs",
            "--userconfig",
            "/opt/workboard-node-profile/empty.npmrc",
            "--globalconfig",
            "/dev/null",
        ]),
        None,
        1048576,
    )
    .await?;
    if !status.success() {
        return Err(err("adapter_profile_install_failed").with_diagnostic(
            crate::diagnostics::Failure::process(
                crate::diagnostics::Stage::Validation,
                status,
                &[],
                &[],
            ),
        ));
    }
    project_pins(Path::new("/workspace"), profile)?;
    Ok(())
}
fn exit(status: std::process::ExitStatus) -> i32 {
    status.code().unwrap_or(-1)
}
pub(super) async fn validate_node(
    profile: &ValidationProfile,
    qa: bool,
    before: &Tree,
) -> Result<Value> {
    if matches!(profile, ValidationProfile::NodeUnit { .. }) {
        return validate_unit(profile, qa, before).await;
    }
    if matches!(profile, ValidationProfile::NodeVite { .. }) {
        return validate_vite(profile, qa, before).await;
    }
    if matches!(profile, ValidationProfile::NodeBuiltin { .. }) {
        return validate_builtin(profile, qa, before).await;
    }
    // Freshen dependencies after model access; Coder cannot poison QA's separate
    // image cache, and neither role can use model-edited node_modules as proof.
    install(profile).await?;
    let imports = "await import('@openai/codex-sdk'); await import('@slack/bolt'); await import('@slack/socket-mode'); await import('@slack/web-api');";
    let (import_status, _, _) = super::contained::local_result(
        "/usr/local/bin/node",
        args(&["--input-type=module", "-e", imports]),
        None,
        131072,
    )
    .await?;
    project_pins(Path::new("/workspace"), profile)?;
    let (syntax_status, _, _) = super::contained::local_result(
        "/usr/local/bin/npm",
        args(&[
            "run",
            "check",
            "--ignore-scripts",
            "--offline",
            "--logs-dir",
            "/tmp/npm-logs",
            "--userconfig",
            "/opt/workboard-node-profile/empty.npmrc",
            "--globalconfig",
            "/dev/null",
        ]),
        None,
        131072,
    )
    .await?;
    project_pins(Path::new("/workspace"), profile)?;
    let (test_status, output, _) = super::contained::local_result(
        "/usr/local/bin/node",
        args(&["--test", "--test-reporter=tap"]),
        None,
        1048576,
    )
    .await?;
    project_pins(Path::new("/workspace"), profile)?;
    let after = source_tree(Path::new("/workspace"))?;
    let head = super::contained::local(
        "/usr/bin/git",
        args(&["-C", "/workspace", "rev-parse", "HEAD"]),
        None,
        65536,
    )
    .await?;
    let counts = tap_counts(&output)?;
    Ok(
        json!({"install_exit_code":0,"dependency_import_exit_code":exit(import_status),"dependency_import_count":4,"syntax_check_exit_code":exit(syntax_status),"test_exit_code":exit(test_status),"tests":counts,"source_before_sha256":before.digest(),"source_after_sha256":after.digest(),"qa_source_unchanged":!qa || before.0==after.0,"head_sha":std::str::from_utf8(&head).unwrap_or("").trim()}),
    )
}
pub(super) fn tap_counts(output: &[u8]) -> Result<Value> {
    let text = std::str::from_utf8(output).map_err(|_| err("adapter_profile_result_invalid"))?;
    let mut counts = BTreeMap::new();
    for line in text.lines() {
        for name in ["tests", "pass", "fail", "cancelled", "skipped", "todo"] {
            if let Some(n) = line.strip_prefix(&format!("# {name} ")) {
                let count = n
                    .parse::<u64>()
                    .ok()
                    .filter(|n| *n <= 1000000)
                    .ok_or_else(|| err("adapter_profile_result_invalid"))?;
                if counts.insert(name, count).is_some() {
                    return Err(err("adapter_profile_result_invalid"));
                }
            }
        }
    }
    if counts.len() != 6
        || counts["tests"]
            != counts["pass"]
                + counts["fail"]
                + counts["cancelled"]
                + counts["skipped"]
                + counts["todo"]
    {
        return Err(err("adapter_profile_result_invalid"));
    }
    Ok(json!(counts))
}
pub(super) fn meaningful(result: &Value) -> bool {
    if !result["syntax_check_count"].is_null() {
        return result["syntax_check_exit_code"] == 0
            && result["syntax_check_count"]
                .as_u64()
                .is_some_and(|n| n > 0 && n <= 512)
            && result["test_exit_code"] == 0
            && result["tests"]["pass"].as_u64().unwrap_or(0) > 0
            && result["tests"]["fail"] == 0
            && result["tests"]["cancelled"] == 0
            && result["qa_source_unchanged"] == true;
    }
    if !result["vite_build_exit_code"].is_null() {
        return result["install_exit_code"] == 0
            && result["typecheck_exit_code"] == 0
            && result["vite_build_exit_code"] == 0
            && result["test_exit_code"] == 0
            && result["tests"]["pass"].as_u64().unwrap_or(0) > 0
            && result["tests"]["fail"] == 0
            && result["tests"]["cancelled"] == 0
            && result["qa_source_unchanged"] == true;
    }
    if !result["review_build_exit_code"].is_null() {
        return result["review_build_exit_code"] == 0
            && result["test_exit_code"] == 0
            && result["tests"]["pass"].as_u64().unwrap_or(0) > 0
            && result["tests"]["fail"] == 0
            && result["tests"]["cancelled"] == 0
            && result["qa_source_unchanged"] == true;
    }
    result["install_exit_code"] == 0
        && result["dependency_import_exit_code"] == 0
        && result["dependency_import_count"] == 4
        && result["syntax_check_exit_code"] == 0
        && result["test_exit_code"] == 0
        && result["tests"]["pass"].as_u64().unwrap_or(0) > 0
        && result["tests"]["fail"] == 0
        && result["tests"]["cancelled"] == 0
        && result["qa_source_unchanged"] == true
}

pub(super) fn evidence(
    value: &Value,
    profile: &ValidationProfile,
    head: &str,
    source_before: &str,
    qa: bool,
) -> Result<()> {
    let bad = || err("adapter_profile_result_invalid");
    let fields = [
        "schema_version",
        "session_id",
        "turn_completed",
        "successful_tests",
        "failed_tests",
        "diff_reviews",
        "commands",
        "profile",
        "validation",
    ];
    if !value
        .as_object()
        .is_some_and(|o| o.len() == fields.len() && fields.iter().all(|f| o.contains_key(*f)))
        || value["schema_version"]
            != if matches!(profile, ValidationProfile::NodeUnit { .. }) {
                5
            } else if matches!(profile, ValidationProfile::NodeVite { .. }) {
                4
            } else if matches!(profile, ValidationProfile::NodeBuiltin { .. }) {
                3
            } else {
                2
            }
        || value["profile"] != serde_json::to_value(profile).map_err(|_| bad())?
        || !value["turn_completed"].is_boolean()
        || !value["session_id"].is_string()
    {
        return Err(bad());
    }
    let validation = &value["validation"];
    let unit = matches!(profile, ValidationProfile::NodeUnit { .. });
    let builtin = matches!(profile, ValidationProfile::NodeBuiltin { .. });
    let vite = matches!(profile, ValidationProfile::NodeVite { .. });
    let fields: Vec<&str> = if unit {
        vec![
            "syntax_check_exit_code",
            "syntax_check_count",
            "test_exit_code",
            "tests",
            "source_before_sha256",
            "source_after_sha256",
            "qa_source_unchanged",
            "head_sha",
        ]
    } else if vite {
        vec![
            "install_exit_code",
            "typecheck_exit_code",
            "vite_build_exit_code",
            "test_exit_code",
            "tests",
            "source_before_sha256",
            "source_after_sha256",
            "qa_source_unchanged",
            "head_sha",
        ]
    } else if builtin {
        vec![
            "review_build_exit_code",
            "test_exit_code",
            "tests",
            "source_before_sha256",
            "source_after_sha256",
            "qa_source_unchanged",
            "head_sha",
        ]
    } else {
        vec![
            "install_exit_code",
            "dependency_import_exit_code",
            "dependency_import_count",
            "syntax_check_exit_code",
            "test_exit_code",
            "tests",
            "source_before_sha256",
            "source_after_sha256",
            "qa_source_unchanged",
            "head_sha",
        ]
    };
    if !validation
        .as_object()
        .is_some_and(|o| o.len() == fields.len() && fields.iter().all(|f| o.contains_key(*f)))
        || validation["head_sha"] != head
        || validation["source_before_sha256"] != source_before
        || !validation["source_after_sha256"]
            .as_str()
            .is_some_and(hex64)
        || !validation["qa_source_unchanged"].is_boolean()
        || (qa
            && (validation["qa_source_unchanged"] != true
                || validation["source_after_sha256"] != source_before))
        || (!unit && !builtin && !vite && validation["dependency_import_count"] != 4)
        || (unit
            && !validation["syntax_check_count"]
                .as_u64()
                .is_some_and(|n| n > 0 && n <= 512))
    {
        return Err(bad());
    }
    for field in if unit {
        vec!["syntax_check_exit_code", "test_exit_code"]
    } else if vite {
        vec![
            "install_exit_code",
            "typecheck_exit_code",
            "vite_build_exit_code",
            "test_exit_code",
        ]
    } else if builtin {
        vec!["review_build_exit_code", "test_exit_code"]
    } else {
        vec![
            "install_exit_code",
            "dependency_import_exit_code",
            "syntax_check_exit_code",
            "test_exit_code",
        ]
    } {
        if validation[field]
            .as_i64()
            .is_none_or(|n| i32::try_from(n).is_err())
        {
            return Err(bad());
        }
    }
    let counts = &validation["tests"];
    let fields = ["tests", "pass", "fail", "cancelled", "skipped", "todo"];
    if !counts
        .as_object()
        .is_some_and(|o| o.len() == fields.len() && fields.iter().all(|f| o.contains_key(*f)))
        || fields
            .iter()
            .any(|f| counts[f].as_u64().is_none_or(|n| n > 1000000))
    {
        return Err(bad());
    }
    if counts["tests"].as_u64().unwrap()
        != ["pass", "fail", "cancelled", "skipped", "todo"]
            .iter()
            .map(|f| counts[f].as_u64().unwrap())
            .sum::<u64>()
    {
        return Err(bad());
    }
    let passed = meaningful(validation);
    if value["successful_tests"] != u64::from(passed)
        || value["failed_tests"] != u64::from(!passed)
        || value["diff_reviews"].as_u64().is_none_or(|n| n > 1)
    {
        return Err(bad());
    }
    let commands = value["commands"]
        .as_array()
        .filter(|a| a.len() <= 256)
        .ok_or_else(bad)?;
    for command in commands {
        if !command.as_object().is_some_and(|o| {
            o.len() == 3
                && ["kind", "exit_code", "completed"]
                    .iter()
                    .all(|f| o.contains_key(*f))
        }) || command["kind"] != "model_command"
            || !command["completed"].is_boolean()
            || (!command["exit_code"].is_null()
                && command["exit_code"]
                    .as_i64()
                    .is_none_or(|n| i32::try_from(n).is_err()))
        {
            return Err(bad());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> ValidationProfile {
        ValidationProfile::Node {
            manifest_sha256: "1".repeat(64),
            lock_sha256: "2".repeat(64),
            check_sha256: "3".repeat(64),
            cache_sha256: "4".repeat(64),
        }
    }
    fn receipt() -> Value {
        json!({"schema_version":2,"session_id":"synthetic-session","turn_completed":true,"successful_tests":1,"failed_tests":0,"diff_reviews":1,"commands":[],"profile":profile(),"validation":{"install_exit_code":0,"dependency_import_exit_code":0,"dependency_import_count":4,"syntax_check_exit_code":0,"test_exit_code":0,"tests":{"tests":2,"pass":1,"fail":0,"cancelled":0,"skipped":1,"todo":0},"source_before_sha256":"5".repeat(64),"source_after_sha256":"5".repeat(64),"qa_source_unchanged":true,"head_sha":"6".repeat(40)}})
    }
    #[test]
    fn absent_legacy_profile_preserves_policy_and_contained_wire_fields() {
        let program = json!({"executable":"/fixed/program","sha256":"a".repeat(64),"credential_env_names":[]});
        let policy = json!({"repository":"fixture/private","state_root":"/fixed/state","auth_home":"/fixed/auth","docker":program,"git":program,"gh":program,"docker_host":"unix:///fixed.sock","worker_image":format!("sha256:{}","a".repeat(64)),"worker_network":"a".repeat(64),"proxy_container":"a".repeat(64),"proxy_image":format!("sha256:{}","a".repeat(64)),"proxy_ip":"192.0.2.2","proxy_port":3128,"proxy_policy_sha256":"a".repeat(64),"proxy_allowed_hosts":["chatgpt.com"],"model":null,"cpus":1,"memory_mb":256});
        let parsed: AdapterPolicy = serde_json::from_value(policy.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), policy);
        let request:Request=serde_json::from_value(json!({"schema_version":1,"action":"coder","action_id":"action","run_id":"run","repository":"fixture/private","task_snapshot":{},"deadline_ms":1,"stage_deadline_ms":1,"workspace":"/fixed/work","base_sha":null,"head_sha":null,"base_ref":null,"qa_receipt":null,"merge_policy":"auto","observed_action_id":null,"approval_sha256":"a".repeat(64),"adapter_policy":policy})).unwrap();
        let wire = super::super::worker::req_to_container(&request).unwrap();
        assert!(wire.get("validation_profile").is_none());
        assert_eq!(wire.as_object().unwrap().len(), 7);
    }
    #[test]
    fn strict_evidence_binds_profile_head_source_steps_and_positive_tests() {
        let value = receipt();
        assert!(evidence(&value, &profile(), &"6".repeat(40), &"5".repeat(64), true).is_ok());
        for (pointer, changed) in [
            ("/schema_version", json!(1)),
            ("/mode", json!("validation_preflight")),
            ("/profile/cache_sha256", json!("0".repeat(64))),
            ("/validation/head_sha", json!("0".repeat(40))),
            ("/validation/source_after_sha256", json!("0".repeat(64))),
            ("/validation/qa_source_unchanged", json!(false)),
            ("/validation/syntax_check_exit_code", Value::Null),
            ("/validation/test_exit_code", json!(1)),
            ("/validation/tests/pass", json!(0)),
            ("/validation/tests/tests", json!(0)),
        ] {
            let mut bad = value.clone();
            if pointer == "/mode" {
                bad["mode"] = changed;
            } else {
                *bad.pointer_mut(pointer).unwrap() = changed;
            }
            assert!(
                evidence(&bad, &profile(), &"6".repeat(40), &"5".repeat(64), true).is_err(),
                "{pointer}"
            );
        }
        let mut negative = value;
        negative["validation"]["syntax_check_exit_code"] = json!(1);
        negative["successful_tests"] = json!(0);
        negative["failed_tests"] = json!(1);
        assert!(
            evidence(
                &negative,
                &profile(),
                &"6".repeat(40),
                &"5".repeat(64),
                true
            )
            .is_ok()
        );
        assert!(!meaningful(&negative["validation"]));
    }
    #[test]
    fn project_pins_refuse_lock_drift_symlinks_and_untracked_qa_changes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("scripts")).unwrap();
        for (file, bytes) in [
            (
                "package.json",
                r#"{"scripts":{"check":"node scripts/check.js","test":"node --test"}}"#,
            ),
            ("package-lock.json", r#"{"lockfileVersion":3}"#),
            ("scripts/check.js", "process.exit(0);\n"),
        ] {
            std::fs::write(root.join(file), bytes).unwrap();
        }
        let pins = ValidationProfile::Node {
            manifest_sha256: sha(&std::fs::read(root.join("package.json")).unwrap()),
            lock_sha256: sha(&std::fs::read(root.join("package-lock.json")).unwrap()),
            check_sha256: sha(&std::fs::read(root.join("scripts/check.js")).unwrap()),
            cache_sha256: "a".repeat(64),
        };
        assert!(project_pins(&root, &pins).is_ok());
        let before = source_tree(&root).unwrap().digest();
        std::fs::write(root.join("untracked.js"), "true").unwrap();
        assert_ne!(before, source_tree(&root).unwrap().digest());
        std::fs::remove_file(root.join("untracked.js")).unwrap();
        std::fs::write(root.join("package-lock.json"), "changed").unwrap();
        assert!(project_pins(&root, &pins).is_err());
        #[cfg(unix)]
        {
            std::fs::remove_file(root.join("package-lock.json")).unwrap();
            std::os::unix::fs::symlink(root.join("package.json"), root.join("package-lock.json"))
                .unwrap();
            assert!(project_pins(&root, &pins).is_err());
        }
    }
    #[test]
    fn finite_profile_and_tap_counts_fail_closed() {
        assert!(serde_json::from_value::<ValidationProfile>(json!({"id":"unknown"})).is_err());
        assert!(
            serde_json::from_value::<ValidationProfile>(
                json!({"id":"python-unittest-v1","command":"anything"})
            )
            .is_err()
        );
        assert!(tap_counts(b"echo '# tests 1'\n# pass 1\n").is_err());
        assert!(
            tap_counts(
                b"# tests 1\n# pass 1\n# fail 0\n# cancelled 0\n# skipped 0\n# todo 0\n# pass 1\n"
            )
            .is_err()
        );
        let counts =
            tap_counts(b"# tests 3\n# pass 1\n# fail 0\n# cancelled 0\n# skipped 1\n# todo 1\n")
                .unwrap();
        assert_eq!(counts["pass"], 1);
        for (passed, syntax, exited) in [(0, 0, 0), (1, 1, 0), (1, 0, 1)] {
            let v = json!({"install_exit_code":0,"dependency_import_exit_code":0,"dependency_import_count":4,"syntax_check_exit_code":syntax,"test_exit_code":exited,"tests":{"pass":passed,"fail":0,"cancelled":0},"qa_source_unchanged":true});
            assert!(!meaningful(&v));
        }
    }
}

async fn validate_builtin(profile: &ValidationProfile, qa: bool, before: &Tree) -> Result<Value> {
    install(profile).await?;
    builtin_output(Path::new("/workspace"))?;
    let build = super::contained::local_result(
        "/usr/local/bin/node",
        args(&["scripts/build.js"]),
        None,
        1048576,
    )
    .await?;
    project_pins(Path::new("/workspace"), profile)?;
    let mut tests = std::fs::read_dir("/workspace/test")
        .map_err(|_| err("adapter_profile_invalid"))?
        .map(|e| e.map_err(|_| err("adapter_profile_invalid")))
        .collect::<Result<Vec<_>>>()?;
    tests.sort_by_key(|e| e.file_name());
    let mut argv = args(&["--test", "--test-reporter=tap"]);
    for entry in tests {
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| err("adapter_profile_invalid"))?;
        if name.ends_with(".test.js")
            && entry
                .file_type()
                .map_err(|_| err("adapter_profile_invalid"))?
                .is_file()
        {
            argv.push(format!("test/{name}"));
        }
    }
    if argv.len() == 2 || argv.len() > 258 {
        return Err(err("adapter_profile_invalid"));
    }
    let tests = super::contained::local_result("/usr/local/bin/node", argv, None, 1048576).await?;
    project_pins(Path::new("/workspace"), profile)?;
    if Path::new("/workspace/dist").symlink_metadata().is_ok() {
        tokio::fs::remove_dir_all("/workspace/dist")
            .await
            .map_err(|_| err("adapter_profile_invalid"))?;
    }
    let after = source_tree(Path::new("/workspace"))?;
    let head =
        super::contained::local("/usr/bin/git", args(&["rev-parse", "HEAD"]), None, 65536).await?;
    Ok(
        json!({"review_build_exit_code":exit(build.0),"test_exit_code":exit(tests.0),"tests":tap_counts(&tests.1)?,"source_before_sha256":before.digest(),"source_after_sha256":after.digest(),"qa_source_unchanged":!qa||before.digest()==after.digest(),"head_sha":std::str::from_utf8(&head).unwrap_or("").trim()}),
    )
}

fn builtin_output(root: &Path) -> Result<()> {
    match root.join("dist").symlink_metadata() {
        Ok(m) if m.is_dir() => Ok(()),
        Ok(_) => Err(err("adapter_profile_invalid")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(err("adapter_profile_invalid")),
    }
}
#[cfg(test)]
mod builtin_tests {
    use super::*;
    #[test]
    fn model_build_output_is_disposable_but_symlinks_are_not() {
        let t = tempfile::tempdir().unwrap();
        assert!(builtin_output(t.path()).is_ok());
        std::fs::create_dir(t.path().join("dist")).unwrap();
        std::fs::write(
            t.path().join("dist/index.html"),
            "synthetic generated review UI",
        )
        .unwrap();
        assert!(builtin_output(t.path()).is_ok());
        std::fs::remove_dir_all(t.path().join("dist")).unwrap();
        std::os::unix::fs::symlink("/private/tmp", t.path().join("dist")).unwrap();
        assert!(builtin_output(t.path()).is_err());
    }
}

#[cfg(test)]
mod builtin_evidence_tests {
    use super::*;
    #[test]
    fn review_build_and_positive_tap_count_are_required_and_bound_to_the_profile() {
        let p = ValidationProfile::NodeBuiltin {
            manifest_sha256: "a".repeat(64),
            build_sha256: "b".repeat(64),
        };
        let mut v = json!({"schema_version":3,"session_id":"synthetic-session","turn_completed":true,"successful_tests":1,"failed_tests":0,"diff_reviews":1,"commands":[],"profile":p,"validation":{"review_build_exit_code":0,"test_exit_code":0,"tests":{"tests":2,"pass":2,"fail":0,"cancelled":0,"skipped":0,"todo":0},"source_before_sha256":"c".repeat(64),"source_after_sha256":"c".repeat(64),"qa_source_unchanged":true,"head_sha":"d".repeat(40)}});
        assert!(evidence(&v, &p, &"d".repeat(40), &"c".repeat(64), true).is_ok());
        for key in ["review_build_exit_code", "test_exit_code"] {
            let mut bad = v.clone();
            bad["validation"][key] = json!(1);
            assert!(!meaningful(&bad["validation"]));
            assert!(evidence(&bad, &p, &"d".repeat(40), &"c".repeat(64), true).is_err());
        }
        v["validation"]["tests"] =
            json!({"tests":2,"pass":0,"fail":0,"cancelled":0,"skipped":2,"todo":0});
        assert!(!meaningful(&v["validation"]));
        assert!(evidence(&v, &p, &"d".repeat(40), &"c".repeat(64), true).is_err());
        v["validation"]["tests"] =
            json!({"tests":0,"pass":0,"fail":0,"cancelled":0,"skipped":0,"todo":0});
        assert!(!meaningful(&v["validation"]));
    }
}

/// Only npm registry integrity-pinned packages; no Git/local/custom registries or lifecycle scripts.
pub(crate) fn validate_vite_inputs(package: &[u8], lock: &[u8]) -> Result<()> {
    let p: Value = serde_json::from_slice(package).map_err(|_| err("adapter_profile_invalid"))?;
    let l: Value = serde_json::from_slice(lock).map_err(|_| err("adapter_profile_invalid"))?;
    if p["scripts"]["build"] != "vite build"
        || p["scripts"]["test"] != "node --test test/*.test.js"
        || p["scripts"]["check:types"] != "tsc --noEmit"
        || ![2, 3].contains(&l["lockfileVersion"].as_u64().unwrap_or(0))
        || [
            "overrides",
            "workspaces",
            "bundledDependencies",
            "bundleDependencies",
        ]
        .iter()
        .any(|k| !p[k].is_null())
    {
        return Err(err("adapter_profile_invalid"));
    }
    let packages = l["packages"]
        .as_object()
        .filter(|m| !m.is_empty() && m.len() < 10000)
        .ok_or_else(|| err("adapter_profile_invalid"))?;
    for scope in ["dependencies", "devDependencies", "optionalDependencies"] {
        if p[scope] != l["packages"][""][scope] {
            return Err(err("adapter_profile_drift"));
        }
        if let Some(deps) = p[scope].as_object() {
            for version in deps.values() {
                if !version.as_str().is_some_and(|s| {
                    !s.is_empty()
                        && s.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b".^~*<>=|+ -".contains(&b))
                }) {
                    return Err(err("adapter_profile_invalid"));
                }
            }
        }
    }
    for (name, entry) in packages {
        if name.is_empty() {
            continue;
        }
        if !name.starts_with("node_modules/")
            || name.contains("..")
            || entry["link"] == true
            || !entry["resolved"].as_str().is_some_and(|s| {
                s.starts_with("https://registry.npmjs.org/")
                    && !s.contains(['?', '#', '\\', '\n', '\r'])
            })
            || !entry["integrity"].as_str().is_some_and(|s| {
                s.starts_with("sha512-")
                    && s.len() < 1024
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"+/=-".contains(&b))
            })
        {
            return Err(err("adapter_profile_invalid"));
        }
    }
    Ok(())
}
async fn validate_vite(profile: &ValidationProfile, qa: bool, before: &Tree) -> Result<Value> {
    install(profile).await?;
    builtin_output(Path::new("/workspace"))?;
    let types = super::contained::local_result(
        "/usr/local/bin/node",
        args(&["node_modules/typescript/bin/tsc", "--noEmit"]),
        None,
        1048576,
    )
    .await?;
    project_pins(Path::new("/workspace"), profile)?;
    let build = super::contained::local_result(
        "/usr/local/bin/node",
        args(&["node_modules/vite/bin/vite.js", "build"]),
        None,
        1048576,
    )
    .await?;
    project_pins(Path::new("/workspace"), profile)?;
    let tests = super::contained::local_result(
        "/usr/local/bin/node",
        test_argv(Path::new("/workspace"))?,
        None,
        1048576,
    )
    .await?;
    project_pins(Path::new("/workspace"), profile)?;
    builtin_output(Path::new("/workspace"))?;
    if Path::new("/workspace/dist").exists() {
        tokio::fs::remove_dir_all("/workspace/dist")
            .await
            .map_err(|_| err("adapter_profile_invalid"))?;
    }
    let after = source_tree(Path::new("/workspace"))?;
    let head =
        super::contained::local("/usr/bin/git", args(&["rev-parse", "HEAD"]), None, 65536).await?;
    Ok(
        json!({"install_exit_code":0,"typecheck_exit_code":exit(types.0),"vite_build_exit_code":exit(build.0),"test_exit_code":exit(tests.0),"tests":tap_counts(&tests.1)?,"source_before_sha256":before.digest(),"source_after_sha256":after.digest(),"qa_source_unchanged":!qa||before.digest()==after.digest(),"head_sha":std::str::from_utf8(&head).unwrap_or("").trim()}),
    )
}
fn test_argv(root: &Path) -> Result<Vec<String>> {
    let mut entries = std::fs::read_dir(root.join("test"))
        .map_err(|_| err("adapter_profile_invalid"))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| err("adapter_profile_invalid"))?;
    entries.sort_by_key(|e| e.file_name());
    let mut argv = args(&["--test", "--test-reporter=tap"]);
    for entry in entries {
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| err("adapter_profile_invalid"))?;
        if name.ends_with(".test.js") && entry.file_type().is_ok_and(|m| m.is_file()) {
            argv.push(format!("test/{name}"));
        }
    }
    if argv.len() == 2 || argv.len() > 258 {
        return Err(err("adapter_profile_invalid"));
    }
    Ok(argv)
}

#[cfg(test)]
mod vite_tests {
    use super::*;
    fn inputs() -> (Value, Value) {
        let p = json!({"scripts":{"build":"vite build","test":"node --test test/*.test.js","check:types":"tsc --noEmit"},"dependencies":{"react":"^19.1.0"},"devDependencies":{"vite":"^7.1.0","typescript":"^5.9.0"}});
        let l = json!({"lockfileVersion":3,"packages":{"":{"dependencies":p["dependencies"],"devDependencies":p["devDependencies"]},"node_modules/react":{"resolved":"https://registry.npmjs.org/react/-/react-19.1.0.tgz","integrity":"sha512-YWJjZA=="}}});
        (p, l)
    }
    #[test]
    fn registry_only_lock_and_exact_manifest_are_required_before_image_build() {
        let (p, l) = inputs();
        assert!(validate_vite_inputs(p.to_string().as_bytes(), l.to_string().as_bytes()).is_ok());
        for source in [
            "file:../host",
            "git+https://github.com/example/repo",
            "https://custom.example/package.tgz",
            "https://registry.npmjs.org.evil.example/package.tgz",
        ] {
            let mut bad = l.clone();
            bad["packages"]["node_modules/react"]["resolved"] = json!(source);
            assert!(
                validate_vite_inputs(p.to_string().as_bytes(), bad.to_string().as_bytes()).is_err()
            );
        }
        for (field, value) in [("link", json!(true)), ("integrity", Value::Null)] {
            let mut bad = l.clone();
            bad["packages"]["node_modules/react"][field] = value;
            assert!(
                validate_vite_inputs(p.to_string().as_bytes(), bad.to_string().as_bytes()).is_err()
            );
        }
        let mut mismatch = l;
        mismatch["packages"][""]["dependencies"]["react"] = json!("^20.0.0");
        assert!(
            validate_vite_inputs(p.to_string().as_bytes(), mismatch.to_string().as_bytes())
                .is_err()
        );
    }
    #[test]
    fn vite_evidence_requires_all_fixed_steps_positive_tests_and_unchanged_qa_source() {
        let p = ValidationProfile::NodeVite {
            manifest_sha256: "a".repeat(64),
            lock_sha256: "b".repeat(64),
            cache_sha256: "c".repeat(64),
        };
        let value = json!({"schema_version":4,"session_id":"synthetic-session","turn_completed":true,"successful_tests":1,"failed_tests":0,"diff_reviews":1,"commands":[],"profile":p,"validation":{"install_exit_code":0,"typecheck_exit_code":0,"vite_build_exit_code":0,"test_exit_code":0,"tests":{"tests":4,"pass":3,"fail":0,"cancelled":0,"skipped":1,"todo":0},"source_before_sha256":"d".repeat(64),"source_after_sha256":"d".repeat(64),"qa_source_unchanged":true,"head_sha":"e".repeat(40)}});
        assert!(evidence(&value, &p, &"e".repeat(40), &"d".repeat(64), true).is_ok());
        for key in [
            "install_exit_code",
            "typecheck_exit_code",
            "vite_build_exit_code",
            "test_exit_code",
        ] {
            let mut bad = value.clone();
            bad["validation"][key] = json!(1);
            assert!(!meaningful(&bad["validation"]));
            assert!(evidence(&bad, &p, &"e".repeat(40), &"d".repeat(64), true).is_err());
        }
        let mut skipped = value.clone();
        skipped["validation"]["tests"] =
            json!({"tests":4,"pass":0,"fail":0,"cancelled":0,"skipped":4,"todo":0});
        assert!(!meaningful(&skipped["validation"]));
        let mut changed = value;
        changed["validation"]["source_after_sha256"] = json!("f".repeat(64));
        assert!(evidence(&changed, &p, &"e".repeat(40), &"d".repeat(64), true).is_err());
    }
}

/// Narrow no-install ESM recipe. Package dependencies are not installed or attested.
pub(crate) fn validate_unit_manifest(raw: &[u8]) -> Result<()> {
    let p: Value = serde_json::from_slice(raw).map_err(|_| err("adapter_profile_invalid"))?;
    if p["type"] != "module"
        || p["scripts"]["test"] != "node --test"
        || !p["scripts"]["build"].is_null()
        || !p["workspaces"].is_null()
    {
        return Err(err("adapter_profile_invalid"));
    }
    Ok(())
}

pub(crate) fn validate_unit_source(root: &Path, profile: &ValidationProfile) -> Result<()> {
    if !matches!(profile, ValidationProfile::NodeUnit { .. }) {
        return Err(err("adapter_profile_invalid"));
    }
    project_pins(root, profile)?;
    unit_files(root)?;
    Ok(())
}

// Enumerated source files, never task-provided argv, globs, npm scripts or a shell.
fn unit_files(root: &Path) -> Result<(Vec<String>, Vec<String>)> {
    let tree = source_tree(root)?;
    let mut syntax = Vec::new();
    for name in tree.0.keys() {
        if name.ends_with(".js") || name.ends_with(".mjs") {
            syntax.push(format!("./{name}"));
        }
    }
    if syntax.is_empty() || syntax.len() > 512 {
        return Err(err("adapter_profile_invalid"));
    }
    // Match the declared node --test discovery, including tests outside test/.
    // source_tree already bounds files/bytes and refuses symlinks; no npm hooks run.
    let tests = args(&["--test", "--test-reporter=tap"]);
    Ok((syntax, tests))
}

async fn validate_unit(profile: &ValidationProfile, qa: bool, before: &Tree) -> Result<Value> {
    let root = Path::new("/workspace");
    install(profile).await?;
    let (files, argv) = unit_files(root)?;
    let mut syntax_exit = 0;
    for file in &files {
        let (status, _, _) = super::contained::local_result(
            "/usr/local/bin/node",
            vec!["--check".into(), file.clone()],
            None,
            131072,
        )
        .await?;
        // A signal must remain infrastructure failure even after an earlier ordinary syntax error.
        if syntax_exit == 0 || status.code().is_none() {
            syntax_exit = exit(status);
        }
    }
    let (status, output, _) =
        super::contained::local_result("/usr/local/bin/node", argv, None, 1048576).await?;
    project_pins(root, profile)?;
    let after = source_tree(root)?;
    let head =
        super::contained::local("/usr/bin/git", args(&["rev-parse", "HEAD"]), None, 65536).await?;
    Ok(
        json!({"syntax_check_exit_code":syntax_exit,"syntax_check_count":files.len(),"test_exit_code":exit(status),"tests":tap_counts(&output)?,"source_before_sha256":before.digest(),"source_after_sha256":after.digest(),"qa_source_unchanged":!qa || before.digest()==after.digest(),"head_sha":std::str::from_utf8(&head).unwrap_or("").trim()}),
    )
}

#[cfg(test)]
mod unit_profile_tests {
    use super::*;
    fn fixture() -> tempfile::TempDir {
        let t = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let r = t.path();
        std::fs::create_dir(r.join("test")).unwrap();
        std::fs::write(r.join("package.json"),r#"{"type":"module","scripts":{"test":"node --test"},"dependencies":{"express":"^5.1.0","dotenv":"^16.6.1"}}"#).unwrap();
        // Syntax checking this file must not import dependencies or start a server.
        std::fs::write(
            r.join("server.mjs"),
            "import 'missing-express-dependency'; throw Error('must not execute');\n",
        )
        .unwrap();
        std::fs::write(r.join("test/logic.test.mjs"),"import test from 'node:test';import assert from 'node:assert/strict';test('logic',()=>assert.equal(2+2,4));\n").unwrap();
        t
    }
    fn profile(root: &Path) -> ValidationProfile {
        ValidationProfile::NodeUnit {
            manifest_sha256: sha(&std::fs::read(root.join("package.json")).unwrap()),
        }
    }
    #[test]
    fn unit_manifest_and_source_refuse_build_workspace_install_aliases_and_unsafe_files() {
        let t = fixture();
        let r = t.path();
        let p = profile(r);
        assert!(validate_unit_source(r, &p).is_ok());
        let raw = std::fs::read(r.join("package.json")).unwrap();
        for (key, value) in [
            ("build", json!("vite build")),
            ("test", json!("node --test && echo extra")),
        ] {
            let mut v: Value = serde_json::from_slice(&raw).unwrap();
            v["scripts"][key] = value;
            assert!(validate_unit_manifest(v.to_string().as_bytes()).is_err());
        }
        let mut v: Value = serde_json::from_slice(&raw).unwrap();
        v["workspaces"] = json!(["nested"]);
        assert!(validate_unit_manifest(v.to_string().as_bytes()).is_err());
        for name in ["node_modules", "package-lock.json", "npm-shrinkwrap.json"] {
            std::fs::write(r.join(name), "must not install or mint a lock").unwrap();
            assert!(validate_unit_source(r, &p).is_err());
            std::fs::remove_file(r.join(name)).unwrap();
        }
        std::os::unix::fs::symlink("/private/tmp", r.join("test/outside")).unwrap();
        assert!(validate_unit_source(r, &p).is_err());
        std::fs::remove_file(r.join("test/outside")).unwrap();
        std::fs::write(r.join("package.json"), "changed").unwrap();
        assert!(project_pins(r, &p).is_err());
    }
    #[test]
    fn unit_file_enumeration_is_bounded_and_passes_filenames_as_data() {
        let t = fixture();
        let r = t.path();
        std::fs::write(r.join("--eval.mjs"), "export const value=1;\n").unwrap();
        std::fs::write(r.join("test/other.test.js"), "// ordinary test file\n").unwrap();
        let (syntax, tests) = unit_files(r).unwrap();
        assert!(syntax.contains(&"./server.mjs".into()));
        assert!(syntax.contains(&"./--eval.mjs".into()));
        assert_eq!(&tests[..2], ["--test", "--test-reporter=tap"]);
        assert_eq!(tests.len(), 2);
        std::fs::remove_dir_all(r.join("test")).unwrap();
        assert_eq!(
            unit_files(r).unwrap().1,
            args(&["--test", "--test-reporter=tap"])
        );
        std::fs::create_dir(r.join("test")).unwrap();
        std::fs::write(r.join("test/a.test.mjs"), "// test\n").unwrap();
        for n in 0..513 {
            std::fs::write(r.join(format!("file{n}.mjs")), "// bounded\n").unwrap();
        }
        assert!(unit_files(r).is_err());
    }
    #[test]
    fn fixed_node_unit_commands_measure_tests_without_executing_server_or_installing() {
        let node = [
            "/usr/local/bin/node",
            "/opt/homebrew/bin/node",
            "/usr/bin/node",
        ]
        .into_iter()
        .find(|p| Path::new(p).is_file())
        .expect("Node is required for this focused test");
        let t = fixture();
        let r = t.path();
        let (syntax, argv) = unit_files(r).unwrap();
        for file in &syntax {
            assert!(
                std::process::Command::new(node)
                    .args(["--check", file])
                    .current_dir(r)
                    .env_clear()
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
        let output = std::process::Command::new(node)
            .args(&argv)
            .current_dir(r)
            .env_clear()
            .output()
            .unwrap();
        assert!(output.status.success());
        let counts = tap_counts(&output.stdout).unwrap();
        assert_eq!(counts["pass"], 1);
        assert_eq!(counts["tests"], 1);
        assert!(!r.join("node_modules").exists());
        assert!(!r.join("package-lock.json").exists());
        // Default discovery must include a failing test outside test/.
        std::fs::create_dir(r.join("src")).unwrap();
        std::fs::write(r.join("src/regression.test.mjs"), "import test from 'node:test';test('outside test directory',()=>{throw Error('regression');});\n").unwrap();
        let outside = std::process::Command::new(node)
            .args(&argv)
            .current_dir(r)
            .env_clear()
            .output()
            .unwrap();
        assert!(!outside.status.success());
        assert_eq!(tap_counts(&outside.stdout).unwrap()["fail"], 1);
        std::fs::remove_file(r.join("src/regression.test.mjs")).unwrap();
        // A newly required missing dependency genuinely fails, rather than triggering npm/network.
        std::fs::write(
            r.join("test/logic.test.mjs"),
            "import 'missing-unit-profile-dependency';\n",
        )
        .unwrap();
        let output = std::process::Command::new(node)
            .args(argv)
            .current_dir(r)
            .env_clear()
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(tap_counts(&output.stdout).unwrap()["fail"], 1);
        std::fs::write(r.join("server.mjs"), "const broken = ;\n").unwrap();
        assert!(
            !std::process::Command::new(node)
                .args(["--check", "./server.mjs"])
                .current_dir(r)
                .env_clear()
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    #[test]
    fn unit_evidence_binds_positive_counts_syntax_exact_head_and_unchanged_qa_source() {
        let p = ValidationProfile::NodeUnit {
            manifest_sha256: "a".repeat(64),
        };
        let v = json!({"schema_version":5,"session_id":"synthetic","turn_completed":true,"successful_tests":1,"failed_tests":0,"diff_reviews":1,"commands":[],"profile":p,"validation":{"syntax_check_exit_code":0,"syntax_check_count":3,"test_exit_code":0,"tests":{"tests":10,"pass":10,"fail":0,"cancelled":0,"skipped":0,"todo":0},"source_before_sha256":"b".repeat(64),"source_after_sha256":"b".repeat(64),"qa_source_unchanged":true,"head_sha":"c".repeat(40)}});
        assert!(evidence(&v, &p, &"c".repeat(40), &"b".repeat(64), true).is_ok());
        for (key, value) in [
            ("syntax_check_exit_code", json!(1)),
            ("syntax_check_exit_code", json!(-1)),
            ("test_exit_code", json!(-1)),
            ("syntax_check_count", json!(0)),
            ("head_sha", json!("d".repeat(40))),
            ("source_after_sha256", json!("e".repeat(64))),
        ] {
            let mut bad = v.clone();
            bad["validation"][key] = value;
            assert!(
                evidence(&bad, &p, &"c".repeat(40), &"b".repeat(64), true).is_err(),
                "{key}"
            );
        }
        for counts in [
            json!({"tests":0,"pass":0,"fail":0,"cancelled":0,"skipped":0,"todo":0}),
            json!({"tests":10,"pass":0,"fail":0,"cancelled":0,"skipped":10,"todo":0}),
        ] {
            let mut bad = v.clone();
            bad["validation"]["tests"] = counts;
            assert!(!meaningful(&bad["validation"]));
            assert!(evidence(&bad, &p, &"c".repeat(40), &"b".repeat(64), true).is_err());
        }
    }
}
