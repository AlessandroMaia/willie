//! Command lines for `wsl.exe`.
//!
//! Building the argument vector is separated from spawning so the exact
//! invocation can be tested and printed (`--dry-run` style) without a WSL
//! installation.

use std::{path::Path, process::Command, str::FromStr};

use crate::{error::WslError, text::decode_wsl_output};

/// Name under which the Willie distribution is registered.
pub const DISTRO_NAME: &str = "willie";

/// Unprivileged user the daemon runs as inside the distribution.
pub const DAEMON_USER: &str = "willie";

/// Absolute path of the daemon binary inside the distribution.
pub const DAEMON_PATH: &str = "/opt/willie/bin/willied";

/// A `wsl.exe` invocation that runs one program inside a distribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslExec {
    distro: String,
    user: Option<String>,
    program: String,
    args: Vec<String>,
}

impl WslExec {
    /// Runs `program` inside `distro` as the distribution's default user.
    #[must_use]
    pub fn new(distro: impl Into<String>, program: impl Into<String>) -> Self {
        Self {
            distro: distro.into(),
            user: None,
            program: program.into(),
            args: Vec::new(),
        }
    }

    /// Runs as an explicit user (`--user`).
    #[must_use]
    pub fn user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }

    /// Appends a program argument.
    #[must_use]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Arguments to pass to `wsl.exe`, in order. `--exec` skips the login
    /// shell so the program's stdio is exactly what the engine sees.
    #[must_use]
    pub fn to_args(&self) -> Vec<String> {
        let mut out = vec!["-d".to_owned(), self.distro.clone()];
        if let Some(user) = &self.user {
            out.push("--user".to_owned());
            out.push(user.clone());
        }
        out.push("--exec".to_owned());
        out.push(self.program.clone());
        out.extend(self.args.iter().cloned());
        out
    }

    /// The invocation that starts the daemon over stdio.
    #[must_use]
    pub fn daemon_stdio() -> Self {
        Self::new(DISTRO_NAME, DAEMON_PATH)
            .user(DAEMON_USER)
            .arg("--stdio")
    }
}

/// Version reported by `wsl.exe --version`, e.g. `2.6.1.0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct WslVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub build: u32,
}

impl WslVersion {
    /// Tar-based distributions and `--install --from-file` need this.
    pub const MINIMUM: Self = Self {
        major: 2,
        minor: 4,
        patch: 4,
        build: 0,
    };

    #[must_use]
    pub fn meets_minimum(&self) -> bool {
        *self >= Self::MINIMUM
    }

    /// Finds the version in the first line of `wsl --version`, whatever
    /// the label's language: the last whitespace-separated token that is
    /// made of digits and dots.
    pub fn parse_report(report: &str) -> Result<Self, WslError> {
        let first = report
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or_default();
        first
            .split_whitespace()
            .rev()
            .find_map(|token| token.parse::<Self>().ok())
            .ok_or_else(|| WslError::Unparseable {
                what: "wsl --version",
                text: first.to_owned(),
            })
    }
}

impl FromStr for WslVersion {
    type Err = WslError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.trim().split('.').map(|p| p.parse::<u32>());
        let mut next = || parts.next().and_then(Result::ok);
        match (next(), next(), next(), next()) {
            (Some(major), Some(minor), Some(patch), build) => Ok(Self {
                major,
                minor,
                patch,
                build: build.unwrap_or(0),
            }),
            _ => Err(WslError::Unparseable {
                what: "version",
                text: s.to_owned(),
            }),
        }
    }
}

/// Recognises the failure `wsl.exe` reports when nothing is registered,
/// whatever the language of the sentence around the code.
#[must_use]
fn is_no_distributions(text: &str) -> bool {
    text.contains("WSL_E_DEFAULT_DISTRO_NOT_FOUND")
}

/// Parses `wsl --list --quiet` / `--list --running --quiet`: one name per
/// line, blank lines ignored, a default marker `*` stripped.
#[must_use]
pub fn parse_name_list(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.trim().trim_start_matches('*').trim())
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `wsl.exe` prepared for a background engine: no console window.
pub(crate) fn wsl_command() -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut cmd = Command::new("wsl.exe");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Tar,
    TarGz,
}

impl ExportFormat {
    const fn flag(self) -> &'static str {
        match self {
            Self::Tar => "tar",
            Self::TarGz => "tar.gz",
        }
    }
}

/// Management commands of `wsl.exe` (its own output is UTF-16LE).
#[derive(Debug, Default, Clone, Copy)]
pub struct WslCli;

impl WslCli {
    fn run(&self, args: &[&str]) -> Result<String, WslError> {
        let output = wsl_command().args(args).output().map_err(|e| match e
            .kind()
        {
            std::io::ErrorKind::NotFound => WslError::NotInstalled(e),
            _ => WslError::Io(e),
        })?;
        let stdout = decode_wsl_output(&output.stdout);
        if output.status.success() {
            Ok(stdout)
        } else {
            let stderr = decode_wsl_output(&output.stderr);
            Err(WslError::CommandFailed {
                args: args.join(" "),
                code: output.status.code(),
                stderr: if stderr.trim().is_empty() {
                    stdout
                } else {
                    stderr
                }
                .trim()
                .to_owned(),
            })
        }
    }

    pub fn version(&self) -> Result<WslVersion, WslError> {
        WslVersion::parse_report(&self.run(&["--version"])?)
    }

    /// With no distribution registered, `wsl --list` reports
    /// WSL_E_DEFAULT_DISTRO_NOT_FOUND and may exit non-zero; that is an
    /// empty list, not a failure.
    fn names(&self, args: &[&str]) -> Result<Vec<String>, WslError> {
        match self.run(args) {
            Ok(text) => Ok(parse_name_list(&text)),
            // `run` folds stdout into `stderr` when stderr is empty, so
            // this one field carries the whole combined output.
            Err(WslError::CommandFailed { stderr, .. })
                if is_no_distributions(&stderr) =>
            {
                Ok(Vec::new())
            }
            Err(err) => Err(err),
        }
    }

    pub fn list(&self) -> Result<Vec<String>, WslError> {
        self.names(&["--list", "--quiet"])
    }

    pub fn running(&self) -> Result<Vec<String>, WslError> {
        self.names(&["--list", "--running", "--quiet"])
    }

    pub fn import(
        &self,
        name: &str,
        location: &Path,
        tar: &Path,
    ) -> Result<(), WslError> {
        let location = location.to_string_lossy();
        let tar = tar.to_string_lossy();
        self.run(&["--import", name, &location, &tar, "--version", "2"])
            .map(drop)
    }

    pub fn unregister(&self, name: &str) -> Result<(), WslError> {
        self.run(&["--unregister", name]).map(drop)
    }

    pub fn terminate(&self, name: &str) -> Result<(), WslError> {
        self.run(&["--terminate", name]).map(drop)
    }

    pub fn export(
        &self,
        name: &str,
        file: &Path,
        format: ExportFormat,
    ) -> Result<(), WslError> {
        let file = file.to_string_lossy();
        self.run(&["--export", name, &file, "--format", format.flag()])
            .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_read_from_a_localised_first_line() {
        let v = WslVersion::parse_report(
            "Versão do WSL: 2.6.1.0\nVersão do kernel: 6.6.87.2-1\n",
        )
        .unwrap();
        assert_eq!(
            v,
            WslVersion {
                major: 2,
                minor: 6,
                patch: 1,
                build: 0
            }
        );
        assert!(v.meets_minimum());
    }

    #[test]
    fn versions_below_the_minimum_are_rejected() {
        assert!(!"2.3.26.0".parse::<WslVersion>().unwrap().meets_minimum());
        assert!("2.4.4".parse::<WslVersion>().unwrap().meets_minimum());
    }

    #[test]
    fn garbage_is_an_unparseable_error() {
        assert!(matches!(
            WslVersion::parse_report("no version here"),
            Err(WslError::Unparseable { .. })
        ));
    }

    #[test]
    fn name_lists_drop_blank_lines_and_default_markers() {
        assert_eq!(
            parse_name_list("\n* Ubuntu\r\nwillie\r\n\n"),
            ["Ubuntu", "willie"]
        );
    }

    #[test]
    fn the_default_distro_error_means_no_distributions() {
        assert!(is_no_distributions(
            "There is no distribution with the supplied name. \
             Error code: Wsl/WSL_E_DEFAULT_DISTRO_NOT_FOUND"
        ));
    }

    #[test]
    fn another_failure_is_not_an_empty_list() {
        assert!(!is_no_distributions(
            "Error code: Wsl/Service/CreateInstance/0x80070569"
        ));
    }

    #[test]
    fn daemon_invocation_runs_unprivileged_over_stdio() {
        let args = WslExec::daemon_stdio().to_args();
        assert_eq!(
            args,
            [
                "-d",
                "willie",
                "--user",
                "willie",
                "--exec",
                "/opt/willie/bin/willied",
                "--stdio",
            ]
        );
    }

    #[test]
    fn user_is_omitted_unless_requested() {
        let args = WslExec::new("willie", "/bin/true").to_args();
        assert!(!args.iter().any(|a| a == "--user"));
    }
}
