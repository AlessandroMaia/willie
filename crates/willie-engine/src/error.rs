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
    #[error("daemon exited with code {code:?}: {stderr}")]
    DaemonExited { code: Option<i32>, stderr: String },
    #[error("protocol violation: {0}")]
    Protocol(String),
    #[error(
        "engine {engine} and daemon {daemon} are different Willie versions"
    )]
    VersionMismatch { engine: String, daemon: String },
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
        }
    }

    /// What the user can do about it.
    #[must_use]
    pub fn remediation(&self) -> String {
        match self {
            Self::Wsl(WslError::CommandFailed { stderr, .. })
                if stderr.contains("0x80070569") =>
            {
                "this account cannot create the WSL 2 VM: an \
                 administrator must grant \"Log on as a service\" to \
                 NT VIRTUAL MACHINE\\Virtual Machines (S-1-5-83-0); \
                 then sign in again"
                    .into()
            }
            Self::Wsl(WslError::NotInstalled(_)) => {
                "enable WSL 2.4.4 or newer (administrator) and retry".into()
            }
            Self::Rpc(e) => e
                .remediation
                .clone()
                .unwrap_or_else(|| "see the daemon log".into()),
            Self::Timeout { .. }
            | Self::Transport(_)
            | Self::DaemonExited { .. } => {
                "restart the daemon from the dashboard".into()
            }
            Self::VersionMismatch { .. } => {
                "reinstall the distribution to update its binaries".into()
            }
            _ => "see the engine log".into(),
        }
    }
}
