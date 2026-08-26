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
