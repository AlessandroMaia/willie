//! Typed errors of the engine. Every variant has a stable code so the UI
//! and the CLI can act on it without parsing messages.

use std::io;

#[derive(Debug, thiserror::Error)]
pub enum WslError {
    #[error("wsl.exe is not installed or not on PATH")]
    NotInstalled(#[source] io::Error),
    #[error("`wsl.exe {args}` failed with {code:?}: {stderr}")]
    CommandFailed {
        args: String,
        code: Option<i32>,
        stderr: String,
    },
    #[error("cannot parse {what} from `{text}`")]
    Unparseable { what: &'static str, text: String },
    #[error("i/o error talking to wsl.exe: {0}")]
    Io(#[from] io::Error),
}

impl WslError {
    /// Stable machine-readable code (see docs/CLI_CONTRACT.md).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotInstalled(_) => "wsl_not_installed",
            Self::CommandFailed { .. } => "wsl_command_failed",
            Self::Unparseable { .. } => "wsl_unparseable_output",
            Self::Io(_) => "wsl_io",
        }
    }
}

use willie_proto::rpc::RpcError;

/// The one host prerequisite a user cannot work around alone. The same
/// text serves whether the missing right surfaced from a `wsl.exe`
/// command or from the daemon's own output, so it is defined once.
const SERVICE_LOGON_REMEDIATION: &str = concat!(
    "this account cannot create the WSL 2 VM: an administrator must ",
    "grant \"Log on as a service\" to NT VIRTUAL MACHINE\\Virtual ",
    "Machines (S-1-5-83-0); then sign in again"
);

/// `Some(127)` is an implementation detail; the user reads a number or
/// nothing. An exit with no output at all is itself the whole story.
fn daemon_exited_message(code: Option<i32>, detail: &str) -> String {
    let code = match code {
        Some(code) => code.to_string(),
        None => "unknown".to_owned(),
    };
    if detail.trim().is_empty() {
        format!("daemon exited with code {code} (no output)")
    } else {
        format!("daemon exited with code {code}: {detail}")
    }
}

/// Errors talking to the daemon from the engine side of the transport.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Wsl(#[from] WslError),
    #[error("daemon returned `{}`: {}", .0.code, .0.message)]
    Rpc(RpcError),
    #[error("no reply to `{method}` within the timeout")]
    Timeout { method: String },
    #[error("transport failure: {0}")]
    Transport(#[from] io::Error),
    #[error("{}", daemon_exited_message(*code, detail))]
    DaemonExited { code: Option<i32>, detail: String },
    #[error("protocol violation: {0}")]
    Protocol(String),
    #[error(
        "engine {engine} and daemon {daemon} are different Willie versions"
    )]
    VersionMismatch { engine: String, daemon: String },
    #[error("distribution image {path} is invalid: {reason}")]
    ImageInvalid { path: String, reason: String },
    #[error("the `willie` distribution is not registered")]
    DistroNotRegistered,
}

impl EngineError {
    /// Stable machine-readable code (see docs/CLI_CONTRACT.md).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Wsl(e) => e.code(),
            Self::Rpc(_) => "daemon_error",
            Self::Timeout { .. } => "daemon_timeout",
            Self::Transport(_) => "daemon_transport",
            Self::DaemonExited { .. } => "daemon_exited",
            Self::Protocol(_) => "protocol_violation",
            Self::VersionMismatch { .. } => "version_mismatch",
            Self::ImageInvalid { .. } => "image_invalid",
            Self::DistroNotRegistered => "distro_not_registered",
        }
    }

    /// What the user can do about it.
    #[must_use]
    pub fn remediation(&self) -> String {
        match self {
            Self::Wsl(WslError::CommandFailed { stderr, .. })
                if stderr.contains("0x80070569") =>
            {
                SERVICE_LOGON_REMEDIATION.into()
            }
            Self::Wsl(WslError::CommandFailed { args, .. })
                if args.starts_with("--import") =>
            {
                "the previous distribution was removed before this \
                 import failed; the image is still on disk — retry \
                 Install"
                    .into()
            }
            Self::Wsl(WslError::NotInstalled(_)) => {
                "enable WSL 2.4.4 or newer (administrator) and retry".into()
            }
            Self::Rpc(e) => e
                .remediation
                .clone()
                .unwrap_or_else(|| "see the daemon log".into()),
            Self::Timeout { .. } | Self::Transport(_) => {
                "restart the daemon from the dashboard".into()
            }
            Self::DaemonExited { detail, .. } => {
                daemon_exited_remediation(detail)
            }
            Self::VersionMismatch { .. } => {
                "reinstall the distribution to update its binaries".into()
            }
            Self::ImageInvalid { .. } => {
                "rebuild the image (just distro-build) and retry".into()
            }
            Self::DistroNotRegistered => "click Install distribution".into(),
            _ => "see the engine log".into(),
        }
    }
}

/// What the daemon's own last words say about who can fix it: a missing
/// host right needs an administrator, a missing distribution needs an
/// install, anything else is worth one more attempt.
fn daemon_exited_remediation(detail: &str) -> String {
    if detail.contains("0x80070569") {
        SERVICE_LOGON_REMEDIATION.into()
    } else if detail.contains("WSL_E_DISTRO_NOT_FOUND") {
        "the `willie` distribution is not registered: click Install \
         distribution"
            .into()
    } else {
        "click Run doctor again (it restarts the daemon); if it repeats, \
         click Install distribution to reinstall it"
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logon_type_error_gets_the_host_prerequisite_remediation() {
        let err = EngineError::Wsl(WslError::CommandFailed {
            args: "--exec /opt/willie/bin/willied --stdio".into(),
            code: Some(1),
            stderr: "Error code: Wsl/Service/CreateInstance/0x80070569".into(),
        });
        assert!(err.remediation().contains("S-1-5-83-0"));
    }

    #[test]
    fn a_failed_import_says_the_image_is_still_on_disk() {
        let err = EngineError::Wsl(WslError::CommandFailed {
            args: "--import willie C:\\dir C:\\image.tar.gz --version 2".into(),
            code: Some(1),
            stderr: "boom".into(),
        });
        assert!(err.remediation().contains("retry Install"));
    }

    fn exited(code: Option<i32>, detail: &str) -> EngineError {
        EngineError::DaemonExited {
            code,
            detail: detail.to_owned(),
        }
    }

    #[test]
    fn an_exited_daemon_shows_the_code_as_a_number_and_its_detail() {
        let err = exited(Some(127), "no distribution with that name");
        assert_eq!(
            err.to_string(),
            "daemon exited with code 127: no distribution with that name"
        );
    }

    #[test]
    fn an_unknown_exit_code_is_named_unknown_not_none() {
        let err = exited(None, "boom");
        assert_eq!(err.to_string(), "daemon exited with code unknown: boom");
    }

    #[test]
    fn a_silent_exit_says_there_was_no_output() {
        assert_eq!(
            exited(Some(1), "  \n").to_string(),
            "daemon exited with code 1 (no output)"
        );
    }

    #[test]
    fn an_exit_naming_the_logon_right_reuses_the_host_remediation() {
        let err = exited(Some(1), "Wsl/Service/CreateInstance/0x80070569");
        assert_eq!(err.remediation(), SERVICE_LOGON_REMEDIATION);
    }

    #[test]
    fn an_exit_naming_a_missing_distribution_points_at_install() {
        let err = exited(Some(127), "Wsl/Service/WSL_E_DISTRO_NOT_FOUND");
        assert!(
            err.remediation().contains("not registered: click Install"),
            "{}",
            err.remediation()
        );
    }

    #[test]
    fn any_other_exit_asks_for_one_more_doctor_run() {
        let err = exited(Some(1), "willied: panicked");
        assert!(err.remediation().starts_with("click Run doctor again"));
    }
}
