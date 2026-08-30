//! Harnesses are the agent CLIs Willie launches and supervises.
//!
//! Every harness is described by a *capability matrix*: plain data that
//! tells the rest of the system what the CLI can do (resume a session,
//! stream headless output, …). Consumers branch on the matrix, never on
//! the harness id, so adding a second harness touches one file.

/// How a harness resumes a previous conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resume {
    /// The harness cannot resume; a new session starts from scratch.
    None,
    /// The harness resumes a conversation given its own session id.
    ById,
    /// The harness resumes the most recent conversation for the project.
    Continue,
}

/// How to start the harness for one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    /// A brand-new conversation.
    Fresh,
    /// Continue the most recent conversation in the workspace.
    Continue,
}

/// What a harness can do. Data, not behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarnessCapabilities {
    /// Runs as an interactive terminal UI.
    pub interactive_tui: bool,
    /// How sessions can be resumed.
    pub resume: Resume,
    /// Supports a non-interactive mode that streams structured output.
    pub headless_stream: bool,
}

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// A harness binary found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub path: PathBuf,
    pub version: String,
}

/// Everything the supervisor needs to exec the harness: `argv[0]` is the
/// absolute binary path, `env` is complete and closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub argv: Vec<String>,
    pub env: BTreeMap<String, String>,
}

/// How long `--version` may take before the binary counts as absent.
const DETECT_TIMEOUT: Duration = Duration::from_secs(5);

/// An agent CLI Willie knows how to run.
pub trait Harness: std::fmt::Debug {
    /// Stable identifier, e.g. `claude-code`.
    fn id(&self) -> &'static str;
    /// Executable name looked up inside the distro.
    fn binary_name(&self) -> &'static str;
    /// Capability matrix.
    fn capabilities(&self) -> HarnessCapabilities;

    /// The version in the binary's `--version` output: the first
    /// whitespace-separated token shaped `N.N.N`.
    fn parse_version(&self, output: &str) -> Option<String> {
        output
            .split_whitespace()
            .map(|t| t.trim_matches(|c: char| !c.is_ascii_digit()))
            .find(|t| {
                let parts: Vec<&str> = t.split('.').collect();
                parts.len() == 3
                    && parts.iter().all(|p| {
                        !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())
                    })
            })
            .map(str::to_owned)
    }

    /// Runs `binary --version` with a timeout. `None` when the binary is
    /// missing, fails, hangs or prints no version.
    fn detect(&self, binary: &Path) -> Option<Installed> {
        let mut child = Command::new(binary)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let deadline = Instant::now() + DETECT_TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        };
        if !status.success() {
            return None;
        }
        let mut out = String::new();
        if let Some(mut stdout) = child.stdout.take() {
            use std::io::Read;
            let _ = stdout.read_to_string(&mut out);
        }
        let version = self.parse_version(&out)?;
        Some(Installed {
            path: binary.to_path_buf(),
            version,
        })
    }

    /// The launch for one session. The environment is an allowlist even
    /// without a sandbox, so the sandbox feature only wraps the child.
    fn launch(
        &self,
        binary: &Path,
        workspace: &Path,
        home: &Path,
        mode: LaunchMode,
    ) -> Launch {
        let _ = workspace;
        let mut env = BTreeMap::new();
        let path = session_path(home)
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(":");
        env.insert("PATH".to_owned(), path);
        env.insert("HOME".to_owned(), home.to_string_lossy().into_owned());
        env.insert(
            "USER".to_owned(),
            home.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "willie".to_owned()),
        );
        env.insert("TERM".to_owned(), "xterm-256color".to_owned());
        env.insert("COLORTERM".to_owned(), "truecolor".to_owned());
        env.insert("LANG".to_owned(), "C.UTF-8".to_owned());
        if let Ok(tz) = std::env::var("TZ")
            && !tz.is_empty()
        {
            env.insert("TZ".to_owned(), tz);
        }
        env.insert("DISABLE_AUTOUPDATER".to_owned(), "1".to_owned());
        let mut argv = vec![binary.to_string_lossy().into_owned()];
        if mode == LaunchMode::Continue {
            argv.push("--continue".to_owned());
        }
        Launch { argv, env }
    }

    /// The official installer, as one `sh -c` command line. Only ever run
    /// on the user's explicit action.
    fn installer(&self) -> &'static str;
}

/// Where a session (and detection) looks for binaries, in order.
#[must_use]
pub fn session_path(home: &Path) -> Vec<PathBuf> {
    // These are Linux paths inside the distro, always `/`-separated, even
    // when this crate is compiled on a Windows host: `Path::join` would
    // insert the host's native separator here instead.
    let mut local_bin = home.to_string_lossy().into_owned();
    local_bin.push_str("/.local/bin");
    vec![
        PathBuf::from(local_bin),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ]
}

/// The first `dir/<binary_name>` that is a file.
#[must_use]
pub fn locate(harness: &dyn Harness, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|d| d.join(harness.binary_name()))
        .find(|p| p.is_file())
}

/// Claude Code, the first supported harness.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClaudeCode;

impl Harness for ClaudeCode {
    fn id(&self) -> &'static str {
        "claude-code"
    }

    fn binary_name(&self) -> &'static str {
        "claude"
    }

    fn capabilities(&self) -> HarnessCapabilities {
        HarnessCapabilities {
            interactive_tui: true,
            resume: Resume::ById,
            headless_stream: true,
        }
    }

    fn installer(&self) -> &'static str {
        "curl -fsSL https://claude.ai/install.sh | bash"
    }
}

/// All harnesses compiled into this build.
#[must_use]
pub fn registry() -> Vec<Box<dyn Harness>> {
    vec![Box::new(ClaudeCode)]
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    use super::*;

    #[test]
    fn harness_ids_are_unique() {
        let ids: HashSet<&str> = registry().iter().map(|h| h.id()).collect();
        assert_eq!(ids.len(), registry().len());
    }

    #[test]
    fn claude_code_resumes_by_id() {
        assert_eq!(ClaudeCode.capabilities().resume, Resume::ById);
        assert_eq!(ClaudeCode.binary_name(), "claude");
    }

    #[test]
    fn the_version_is_the_first_dotted_number_in_the_output() {
        assert_eq!(
            ClaudeCode.parse_version("2.1.246 (Claude Code)\n"),
            Some("2.1.246".to_owned())
        );
        assert_eq!(
            ClaudeCode.parse_version("claude 1.0.7\n"),
            Some("1.0.7".to_owned())
        );
        assert_eq!(ClaudeCode.parse_version("not a version"), None);
        assert_eq!(ClaudeCode.parse_version(""), None);
    }

    #[test]
    fn claude_continue_mode_appends_the_continue_flag() {
        let bin = std::path::Path::new("/opt/willie/bin/claude");
        let ws = std::path::Path::new("/home/willie/projects/x");
        let home = std::path::Path::new("/home/willie");
        let fresh = ClaudeCode.launch(bin, ws, home, LaunchMode::Fresh);
        assert_eq!(fresh.argv, vec![bin.to_string_lossy().into_owned()]);
        let cont = ClaudeCode.launch(bin, ws, home, LaunchMode::Continue);
        assert_eq!(
            cont.argv,
            vec![bin.to_string_lossy().into_owned(), "--continue".to_owned()]
        );
    }

    #[test]
    fn launch_uses_the_binary_as_argv0_and_only_the_allowlisted_env() {
        // TZ is read from this process; make the test deterministic.
        // SAFETY: tests run single-threaded here; nothing reads the env
        // concurrently.
        unsafe { std::env::set_var("TZ", "UTC") };
        let l = ClaudeCode.launch(
            Path::new("/home/willie/.local/bin/claude"),
            Path::new("/home/willie/projects/x"),
            Path::new("/home/willie"),
            LaunchMode::Fresh,
        );
        assert_eq!(l.argv, vec!["/home/willie/.local/bin/claude".to_owned()]);
        let keys: Vec<&str> = l.env.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            vec![
                "COLORTERM",
                "DISABLE_AUTOUPDATER",
                "HOME",
                "LANG",
                "PATH",
                "TERM",
                "TZ",
                "USER"
            ]
        );
        assert_eq!(
            l.env["PATH"],
            "/home/willie/.local/bin:/usr/local/bin:/usr/bin:/bin"
        );
        assert_eq!(l.env["HOME"], "/home/willie");
        assert_eq!(l.env["USER"], "willie");
        assert_eq!(l.env["TERM"], "xterm-256color");
        assert_eq!(l.env["COLORTERM"], "truecolor");
        assert_eq!(l.env["LANG"], "C.UTF-8");
        assert_eq!(l.env["DISABLE_AUTOUPDATER"], "1");
        assert_eq!(l.env["TZ"], "UTC");
    }

    #[test]
    fn the_session_path_starts_in_the_home_and_ends_in_bin() {
        let dirs = session_path(Path::new("/home/willie"));
        assert_eq!(dirs[0], PathBuf::from("/home/willie/.local/bin"));
        assert_eq!(dirs.last().unwrap(), &PathBuf::from("/bin"));
        assert_eq!(dirs.len(), 4);
    }

    #[test]
    fn locate_returns_the_first_directory_holding_the_binary() {
        let root = std::env::temp_dir()
            .join(format!("willie-harness-locate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let a = root.join("a");
        let b = root.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(b.join("claude"), b"").unwrap();
        assert_eq!(
            locate(&ClaudeCode, &[a.clone(), b.clone()]),
            Some(b.join("claude"))
        );
        assert_eq!(locate(&ClaudeCode, &[a]), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn detect_runs_the_binary_and_reads_its_version() {
        let root = std::env::temp_dir()
            .join(format!("willie-harness-detect-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let bin = root.join("claude");
        std::fs::write(&bin, "#!/bin/sh\necho '9.8.7 (fake)'\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .unwrap();
        let found = ClaudeCode.detect(&bin).unwrap();
        assert_eq!(found.version, "9.8.7");
        assert_eq!(found.path, bin);
        assert!(ClaudeCode.detect(&root.join("missing")).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_installer_is_a_shell_command_naming_the_official_source() {
        assert!(ClaudeCode.installer().starts_with("curl "));
        assert!(ClaudeCode.installer().ends_with("| bash"));
    }
}
