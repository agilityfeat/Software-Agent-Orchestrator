//! Bounded, informational failure facts. Never retain raw subprocess output here.
use serde::{Deserialize, Serialize};
use std::process::ExitStatus;
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Stage {
    Driver,
    ArtifactTransfer,
    AuthSeed,
    AuthRead,
    ContainerSetup,
    ContainerInspection,
    SourcePreparation,
    SourceVerification,
    Codex,
    ProviderResult,
    Validation,
    PatchExport,
    Cleanup,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Category {
    ProcessExit,
    Signal,
    Authentication,
    ProxyConnection,
    Connection,
    ConfigurationSchema,
    InvalidResult,
    Transport,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Code {
    ContainedExportInvalid,
    ContainedCommandFailed,
    ContainedResultInvalid,
    ContainedInputInvalid,
    ContainedHeadChanged,
    AdapterWorkerResultInvalid,
    AdapterWorkerResultSchemaInvalid,
    AdapterValidationEvidenceInvalid,
    AdapterValidationSessionMismatch,
    AdapterValidationTurnIncomplete,
    AdapterValidationSuccessMissing,
    AdapterQaIncomplete,
    AdapterQaFeedbackInvalid,
    AdapterQaValidationIncomplete,
    AdapterProfileInvalid,
    AdapterProfileDrift,
    AdapterProfileInstallFailed,
    AdapterProfileResultInvalid,
    AdapterPatchInvalid,
    AdapterAuthRefreshUnknown,
    AdapterAuthRefreshQuarantined,
    AdapterAuthReferenceInvalid,
    AdapterAuthReferenceChanged,
    AdapterCleanupUnknown,
    AdapterCommandFailed,
    AdapterCommandTimeout,
    AdapterStateInvalid,
    AdapterPinInvalid,
    AdapterProxyInvalid,
    DriverTransportFailed,
    DriverReceiptInvalid,
    NativeSetupBlocked,
    NativeValidationIncomplete,
    NativeValidationInvalid,
    NativeAuthRefreshUnknown,
    NativeCleanupUnknown,
    NativeDeadlineExceeded,
    NativeCancelled,
    NativeProcessFailed,
    NativeResultInvalid,
    NativeTurnIncomplete,
    NativeExecutionFailed,
    NativeQaSourceChanged,
    NativeProtectedSourcePath,
    NativeUnignoredEnvironment,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Failure {
    schema_version: u8,
    stage: Stage,
    category: Category,
    exit_code: Option<i32>,
    signal: Option<i32>,
    http_status: Option<u16>,
    session_started: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    code: Option<Code>,
}
impl Failure {
    pub(crate) fn command_stage(program: &str, args: &[String]) -> Stage {
        if args
            .iter()
            .any(|v| ["--export-evidence", "--export-patch"].contains(&v.as_str()))
            || args.iter().any(|v| v == "cp")
        {
            return Stage::ArtifactTransfer;
        }
        if args.iter().any(|v| v == "--export-auth") {
            return Stage::AuthRead;
        }
        if args.iter().any(|v| v == "--stop-worker" || v == "rm") {
            return Stage::Cleanup;
        }
        if args
            .iter()
            .any(|v| v.contains("cat > /tmp/codex/auth.json"))
        {
            return Stage::AuthSeed;
        }
        if args.iter().any(|v| v == "create" || v == "start") {
            return Stage::ContainerSetup;
        }
        if args.iter().any(|v| v == "inspect" || v == "ps") {
            return Stage::ContainerInspection;
        }
        if program.ends_with("/codex")
            || args
                .iter()
                .any(|v| v == "/usr/local/bin/workboard-contained")
        {
            Stage::Codex
        } else if args.iter().any(|v| v == "clone") {
            Stage::SourcePreparation
        } else if args.iter().any(|v| v == "rev-parse") {
            Stage::SourceVerification
        } else if args
            .iter()
            .any(|v| v == "--cached" || v == "add" || v == "apply")
        {
            Stage::PatchExport
        } else if program.ends_with("/npm")
            || (program.ends_with("/node")
                && args
                    .iter()
                    .any(|v| v == "--test" || v == "--input-type=module"))
            || args.iter().any(|v| v == "unittest" || v == "diff")
        {
            Stage::Validation
        } else if args.iter().any(|v| v == "--stop-worker" || v == "rm") {
            Stage::Cleanup
        } else {
            Stage::Driver
        }
    }
    pub(crate) fn with_code(mut self, code: &str) -> Self {
        if self.code.is_none() {
            self.code = serde_json::from_value(serde_json::json!(code)).ok();
        }
        self
    }
    pub(crate) fn for_code(code: &str) -> Option<Self> {
        let recognized: Code = serde_json::from_value(serde_json::json!(code)).ok()?;
        let stage = match recognized {
            Code::AdapterProfileInvalid
            | Code::AdapterProfileDrift
            | Code::AdapterProfileInstallFailed
            | Code::AdapterProfileResultInvalid => Stage::Validation,
            Code::ContainedExportInvalid => Stage::ArtifactTransfer,
            Code::ContainedCommandFailed => Stage::Codex,
            Code::ContainedHeadChanged => Stage::SourceVerification,
            Code::ContainedResultInvalid
            | Code::AdapterWorkerResultInvalid
            | Code::AdapterWorkerResultSchemaInvalid
            | Code::AdapterQaIncomplete
            | Code::AdapterQaFeedbackInvalid => Stage::ProviderResult,
            Code::AdapterValidationEvidenceInvalid
            | Code::AdapterValidationSessionMismatch
            | Code::AdapterValidationTurnIncomplete
            | Code::AdapterValidationSuccessMissing
            | Code::AdapterQaValidationIncomplete => Stage::Validation,
            Code::AdapterPatchInvalid => Stage::PatchExport,
            Code::AdapterAuthRefreshUnknown
            | Code::AdapterAuthRefreshQuarantined
            | Code::AdapterAuthReferenceInvalid
            | Code::AdapterAuthReferenceChanged => Stage::AuthRead,
            Code::AdapterCleanupUnknown | Code::NativeCleanupUnknown | Code::NativeCancelled => {
                Stage::Cleanup
            }
            Code::NativeAuthRefreshUnknown => Stage::AuthRead,
            Code::NativeSetupBlocked
            | Code::NativeValidationIncomplete
            | Code::NativeValidationInvalid => Stage::Validation,
            Code::NativeResultInvalid | Code::NativeTurnIncomplete => Stage::ProviderResult,
            Code::NativeQaSourceChanged
            | Code::NativeProtectedSourcePath
            | Code::NativeUnignoredEnvironment => Stage::SourceVerification,
            Code::NativeProcessFailed | Code::NativeDeadlineExceeded => Stage::Codex,
            _ => Stage::Driver,
        };
        Some(Self::new(stage, Category::InvalidResult).with_code(code))
    }
    pub(crate) fn new(stage: Stage, category: Category) -> Self {
        Self {
            schema_version: 1,
            stage,
            category,
            exit_code: None,
            signal: None,
            http_status: None,
            session_started: false,
            code: None,
        }
    }
    pub(crate) fn with_session_started(mut self, started: bool) -> Self {
        self.session_started = started;
        self
    }
    pub(crate) fn process(stage: Stage, status: ExitStatus, stdout: &[u8], stderr: &[u8]) -> Self {
        let mut f = Self::new(stage, Category::ProcessExit);
        f.exit_code = status.code();
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            f.signal = status.signal();
        }
        if f.signal.is_some() {
            f.category = Category::Signal;
        }
        // These are hints from bounded in-memory bytes, never permission or delivery evidence.
        let excerpt = |bytes: &[u8]| {
            let n = bytes.len().min(8192);
            format!(
                "{}\n{}",
                String::from_utf8_lossy(&bytes[..n]),
                String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(n)..])
            )
        };
        let text = format!("{}\n{}", excerpt(stdout), excerpt(stderr)).to_ascii_lowercase();
        for status in [
            400, 401, 403, 407, 408, 409, 413, 422, 429, 500, 502, 503, 504,
        ] {
            if [
                format!("http {status}"),
                format!("status {status}"),
                format!("status code: {status}"),
                format!("status code {status}"),
                format!("status: {status}"),
                format!("{status} unauthorized"),
                format!("{status} forbidden"),
            ]
            .iter()
            .any(|v| text.contains(v))
            {
                f.http_status = Some(status);
                break;
            }
        }
        if matches!(f.http_status, Some(401 | 403))
            || text.contains("not logged in")
            || text.contains("authentication failed")
        {
            f.category = Category::Authentication;
        } else if f.http_status == Some(407)
            || text.contains("proxyconnect")
            || text.contains("proxy connection")
            || text.contains("proxy error")
            || text.contains("tunnel connection failed")
        {
            f.category = Category::ProxyConnection;
        } else if [
            "connection refused",
            "failed to connect",
            "error sending request",
            "failed to send request",
            "dns error",
            "connection timed out",
            "certificate verify failed",
            "unable to get local issuer certificate",
        ]
        .iter()
        .any(|v| text.contains(v))
        {
            f.category = Category::Connection;
        } else if [
            "invalid schema",
            "invalid output schema",
            "unexpected argument",
            "error parsing config",
            "invalid configuration",
        ]
        .iter()
        .any(|v| text.contains(v))
        {
            f.category = Category::ConfigurationSchema;
        }
        f.session_started = stdout.split(|b| *b == b'\n').any(|line| {
            line.len() <= 4096
                && serde_json::from_slice::<serde_json::Value>(line)
                    .is_ok_and(|v| v["type"] == "thread.started")
        });
        f
    }
    pub(crate) fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > 1024 {
            return None;
        }
        let f: Self = serde_json::from_slice(bytes).ok()?;
        if f.schema_version != 1
            || f.http_status.is_some_and(|v| {
                ![
                    400, 401, 403, 407, 408, 409, 413, 422, 429, 500, 502, 503, 504,
                ]
                .contains(&v)
            })
            || f.exit_code.is_some_and(|v| !(0..=255).contains(&v))
            || f.signal.is_some_and(|v| !(1..=64).contains(&v))
        {
            return None;
        }
        Some(f)
    }
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("fixed failure schema")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn failure_projection_discards_private_text_and_rejects_arbitrary_fields() {
        use std::os::unix::process::ExitStatusExt;
        let f = Failure::process(
            Stage::Codex,
            ExitStatus::from_raw(256),
            b"{\"type\":\"thread.started\",\"thread_id\":\"private-session\"}\n",
            b"synthetic-private-token HTTP 401 Unauthorized private prompt",
        );
        let v = f.json();
        assert_eq!(v["category"], "authentication");
        assert_eq!(v["http_status"], 401);
        assert_eq!(v["exit_code"], 1);
        assert_eq!(v["session_started"], true);
        assert!(!v.to_string().contains("private"));
        let mut unsafe_value = v;
        unsafe_value["raw"] = serde_json::json!("must never survive");
        assert!(Failure::parse(unsafe_value.to_string().as_bytes()).is_none());
        assert!(Failure::parse(b"synthetic-private-token").is_none());
    }
}
