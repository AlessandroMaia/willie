//! Command lines for `wsl.exe`.
//!
//! Building the argument vector is separated from spawning so the exact
//! invocation can be tested and printed (`--dry-run` style) without a WSL
//! installation.

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

#[cfg(test)]
mod tests {
    use super::*;

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
