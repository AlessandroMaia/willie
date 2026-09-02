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
    /// Stable machine-readable code (see docs/PROTOCOL.md, Engine
    /// problem codes).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotInstalled(_) => "wsl_not_installed",
            Self::CommandFailed { stderr, .. }
                if stderr.contains(LOGON_TYPE_NOT_GRANTED) =>
            {
                SERVICE_LOGON_RIGHT_CODE
            }
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

/// The HCS code Windows reports when the virtual-machine account lacks
/// that right. It arrives inside `wsl.exe`'s stderr and inside the
/// daemon's own last words, so both paths recognise it by this text.
const LOGON_TYPE_NOT_GRANTED: &str = "0x80070569";

/// The one problem code whose remedy needs an administrator rather than
/// the user. Not for any other `wsl.exe` or daemon failure: seeing it,
/// the UI offers the script below, and that script grants a Windows
/// right.
const SERVICE_LOGON_RIGHT_CODE: &str = "service_logon_right_missing";

/// The commands that grant the right the remediation asks for, ready to
/// paste into an administrator's prompt. It reads the current
/// assignment and rewrites it only when the account is missing, and
/// writes the template back in the encoding `secedit` produced, because
/// it runs elevated on a machine Willie does not control.
pub const SERVICE_LOGON_FIX_SCRIPT: &str = concat!(
    "# Run in an elevated PowerShell (Run as administrator).\n",
    r#"$cfg = "$env:TEMP\willie-secpol.cfg""#,
    "\n",
    r#"$db = "$env:windir\security\local.sdb""#,
    "\n",
    "secedit /export /cfg $cfg /areas USER_RIGHTS | Out-Null\n",
    r"if (-not ((Get-Content $cfg) -match ",
    r"'^SeServiceLogonRight .*\*S-1-5-83-0')) {",
    "\n",
    r"    (Get-Content $cfg) -replace ",
    r"'^(SeServiceLogonRight .*)', '$1,*S-1-5-83-0' |",
    "\n",
    "        Set-Content $cfg -Encoding Unicode\n",
    "    secedit /configure /db $db /cfg $cfg /areas USER_RIGHTS\n",
    "}\n",
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
    #[error("no distribution image next to the app")]
    ImageNotFound,
    #[error("the daemon is not running")]
    DaemonNotRunning,
    #[error("the `willie` distribution is not registered")]
    DistroNotRegistered,
    #[error("path `{path}` does not exist")]
    PathNotFound { path: String },
    #[error("cannot write {path}: {message}")]
    ConfigWrite { path: String, message: String },
    #[error("terminal launch failed: {message}")]
    TerminalLaunch {
        message: String,
        attach_hint: String,
    },
    #[error("embedded terminal failed: {message}")]
    EmbeddedTerminal { message: String },
}

impl EngineError {
    /// Stable machine-readable code (see docs/PROTOCOL.md, Engine
    /// problem codes).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Wsl(e) => e.code(),
            Self::Rpc(_) => "daemon_error",
            Self::Timeout { .. } => "daemon_timeout",
            Self::Transport(_) => "daemon_transport",
            Self::DaemonExited { detail, .. }
                if detail.contains(LOGON_TYPE_NOT_GRANTED) =>
            {
                SERVICE_LOGON_RIGHT_CODE
            }
            Self::DaemonExited { .. } => "daemon_exited",
            Self::Protocol(_) => "protocol_violation",
            Self::VersionMismatch { .. } => "version_mismatch",
            Self::ImageInvalid { .. } => "image_invalid",
            Self::ImageNotFound => "image_not_found",
            Self::DaemonNotRunning => "daemon_not_running",
            Self::DistroNotRegistered => "distro_not_registered",
            Self::PathNotFound { .. } => "path_not_found",
            Self::ConfigWrite { .. } => "config_write_failed",
            Self::TerminalLaunch { .. } => "terminal_launch_failed",
            Self::EmbeddedTerminal { .. } => "embedded_terminal_failed",
        }
    }

    /// What the user can do about it.
    #[must_use]
    pub fn remediation(&self) -> String {
        match self {
            Self::Wsl(WslError::CommandFailed { stderr, .. })
                if stderr.contains(LOGON_TYPE_NOT_GRANTED) =>
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
            Self::Wsl(err) => wsl_remediation(err),
            Self::Rpc(e) => e.remediation.clone().unwrap_or_else(|| {
                "the daemon rejected the request; the message names the \
                 cause — click Run doctor to retry"
                    .to_owned()
            }),
            Self::Timeout { .. } | Self::Transport(_) => {
                "click Run doctor (it restarts the daemon)".into()
            }
            Self::DaemonExited { detail, .. } => {
                daemon_exited_remediation(detail)
            }
            Self::Protocol(_) => {
                "click Run doctor (it restarts the daemon); if it repeats, \
                 click Install distribution"
                    .into()
            }
            Self::VersionMismatch { .. } => {
                "click Install distribution: it reinstalls the \
                 distribution with binaries matching this app"
                    .into()
            }
            Self::ImageInvalid { .. } => {
                "rebuild the image with `just distro-build`, then click \
                 Install distribution again"
                    .into()
            }
            Self::ImageNotFound => {
                "reinstall Willie; in development run `just distro-build`"
                    .into()
            }
            Self::DaemonNotRunning => {
                "click Run doctor (it starts the daemon)".into()
            }
            Self::DistroNotRegistered => "click Install distribution".into(),
            Self::PathNotFound { .. } => {
                "pick a folder that exists on this machine".into()
            }
            Self::ConfigWrite { .. } => "check that Willie can write to \
                 %LOCALAPPDATA%\\Willie\\data"
                .into(),
            Self::TerminalLaunch { attach_hint, .. } => {
                format!("open a terminal and run: {attach_hint}")
            }
            Self::EmbeddedTerminal { .. } => "open the session in a \
                 Windows Terminal tab instead (Open session), or click \
                 Run doctor"
                .into(),
        }
    }
}

/// The `wsl.exe` failures that are neither the missing service-logon
/// right nor a failed import: no log to read, so each names the command
/// the user can run to see the same message.
fn wsl_remediation(err: &WslError) -> String {
    match err {
        WslError::NotInstalled(_) => {
            "enable WSL 2.4.4 or newer (administrator) and retry".into()
        }
        WslError::CommandFailed { .. } => {
            "`wsl.exe` refused the command; run it in PowerShell to see \
             the same message, then click Run doctor again"
                .into()
        }
        WslError::Unparseable { .. } => {
            "unexpected output from `wsl.exe`: run `wsl --version` in \
             PowerShell, update WSL if it is old, then click Run doctor"
                .into()
        }
        WslError::Io(_) => {
            "the pipe to `wsl.exe` broke; click Run doctor (it restarts \
             the daemon)"
                .into()
        }
    }
}

/// What the daemon's own last words say about who can fix it: a missing
/// host right needs an administrator, a missing distribution needs an
/// install, anything else is worth one more attempt.
fn daemon_exited_remediation(detail: &str) -> String {
    if detail.contains(LOGON_TYPE_NOT_GRANTED) {
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

    /// The same host prerequisite reaches the UI from two unrelated
    /// failures, and both must name it with the one code the Dashboard
    /// keys its "copy the fix" action on.
    #[test]
    fn the_missing_logon_right_reports_one_code_from_both_paths() {
        let from_command = EngineError::Wsl(WslError::CommandFailed {
            args: "--exec /opt/willie/bin/willied --stdio".into(),
            code: Some(1),
            stderr: "Wsl/Service/CreateInstance/0x80070569".into(),
        });
        let from_exit =
            exited(Some(1), "Wsl/Service/CreateInstance/0x80070569");

        assert_eq!(from_command.code(), "service_logon_right_missing");
        assert_eq!(from_exit.code(), "service_logon_right_missing");
    }

    /// The specific code may not swallow the generic ones: any other
    /// output is still just a failed command or a dead daemon.
    #[test]
    fn a_failure_without_the_logon_code_keeps_its_generic_code() {
        let err = EngineError::Wsl(WslError::CommandFailed {
            args: "--list --quiet".into(),
            code: Some(1),
            stderr: "boom".into(),
        });

        assert_eq!(err.code(), "wsl_command_failed");
        assert_eq!(exited(Some(127), "boom").code(), "daemon_exited");
    }

    /// The script is what the user pastes into an administrator's
    /// prompt, so it may not drift from the remediation beside it: both
    /// name the same right and the same account, and the script says
    /// which prompt it needs.
    #[test]
    fn the_fix_script_grants_the_right_the_remediation_names() {
        for text in [SERVICE_LOGON_REMEDIATION, SERVICE_LOGON_FIX_SCRIPT] {
            assert!(text.contains("S-1-5-83-0"), "{text}");
        }

        assert!(SERVICE_LOGON_FIX_SCRIPT.contains("SeServiceLogonRight"));
        assert!(SERVICE_LOGON_FIX_SCRIPT.contains("elevated"));
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

    /// One value per variant, the four `WslError` codes included. A new
    /// variant is added here so the invariants below keep covering the
    /// whole enum.
    fn one_of_each_error() -> Vec<EngineError> {
        vec![
            EngineError::Wsl(WslError::NotInstalled(io::Error::other("x"))),
            EngineError::Wsl(WslError::CommandFailed {
                args: "--list --quiet".into(),
                code: Some(1),
                stderr: "boom".into(),
            }),
            EngineError::Wsl(WslError::Unparseable {
                what: "wsl --version",
                text: "no version here".into(),
            }),
            EngineError::Wsl(WslError::Io(io::Error::other("pipe"))),
            EngineError::Rpc(RpcError::new("internal_error", "it broke")),
            EngineError::Timeout {
                method: "daemon.health".into(),
            },
            EngineError::Transport(io::Error::other("pipe")),
            exited(Some(127), "no distribution with that name"),
            EngineError::Protocol("daemon closed its stdout".into()),
            EngineError::VersionMismatch {
                engine: "0.1.0".into(),
                daemon: "0.0.9".into(),
            },
            EngineError::ImageInvalid {
                path: "C:\\x.tar.gz".into(),
                reason: "empty".into(),
            },
            EngineError::ImageNotFound,
            EngineError::DaemonNotRunning,
            EngineError::DistroNotRegistered,
            EngineError::PathNotFound {
                path: r"C:\does\not\exist".into(),
            },
            EngineError::ConfigWrite {
                path: r"C:\LOCALAPPDATA\Willie\data\engine.toml".into(),
                message: "access is denied".into(),
            },
            EngineError::TerminalLaunch {
                message: "boom".into(),
                attach_hint: "wsl -d willie --user willie -- willie \
                              attach sess_1"
                    .into(),
            },
            EngineError::EmbeddedTerminal {
                message: "x".into(),
            },
        ]
    }

    /// The UI keys behaviour on the code, so two failures that need
    /// different handling must never answer with the same one.
    #[test]
    fn every_error_code_is_unique() {
        let mut codes: Vec<&str> =
            one_of_each_error().iter().map(EngineError::code).collect();
        let total = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), total, "duplicate code in {codes:?}");
    }

    /// F0 writes no log file, so no remediation may send the user to
    /// one. "Log on as a service" is the name of a Windows right.
    #[test]
    fn no_remediation_mentions_a_log() {
        const LOG_PHRASES: [&str; 6] = [
            "engine log",
            "daemon log",
            "the log",
            "a log",
            "log file",
            "logs",
        ];
        for err in one_of_each_error() {
            let text = err.remediation().to_lowercase();
            assert!(!text.is_empty(), "{} has no remediation", err.code());
            for phrase in LOG_PHRASES {
                assert!(!text.contains(phrase), "{}: {text}", err.code());
            }
        }
    }

    /// A failed tab is a paste-and-run remediation, not a dead end: the
    /// exact attach line must survive into the problem the UI shows.
    #[test]
    fn a_terminal_launch_failure_maps_to_its_code_and_a_paste_remediation() {
        let p = crate::engine::Problem::from(&EngineError::TerminalLaunch {
            message: "boom".into(),
            attach_hint: "wsl -d willie --user willie -- willie attach \
                          sess_1"
                .into(),
        });
        assert_eq!(p.code, "terminal_launch_failed");
        assert!(p.remediation.contains("willie attach sess_1"));
    }
}
