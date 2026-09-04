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

use willie_core::sandbox::CapabilitySet;

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

/// A symbolic link the sandbox recreates inside the private home, so a
/// dot path resolves into the bound state exactly as it does in the
/// distribution's real home. `target` is what the link says, relative
/// to the home, because that is how the image writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub target: String,
    pub path: PathBuf,
}

/// Where a harness keeps its login and settings: one directory the
/// image places outside the ephemeral home, plus the links that make
/// the harness find it. `agent.state` binds the directory read-write
/// and recreates the links; nothing else of the real home is visible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentState {
    pub dir: PathBuf,
    pub links: Vec<Link>,
}

/// One package cache: the short name a per-project copy is kept under,
/// and the home-relative path the tools expect to find it at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cache {
    pub name: &'static str,
    pub path: PathBuf,
}

/// `home/<rel>` as a Linux path. `Path::join` would insert the host's
/// separator when this crate is compiled on Windows; these paths only
/// ever exist inside the distribution.
fn under(home: &Path, rel: &str) -> PathBuf {
    let mut s = home.to_string_lossy().into_owned();
    s.push('/');
    s.push_str(rel);
    PathBuf::from(s)
}

/// The roots managed tools are installed under, bound read-only by
/// `tools.ro`: the user-local tree the official installers use, and
/// the .NET root. A root that does not exist yet is skipped by the
/// sandbox, so the list names the layout rather than the machine.
#[must_use]
pub fn tool_roots(home: &Path) -> Vec<PathBuf> {
    vec![under(home, ".local"), under(home, ".dotnet")]
}

/// The package caches `caches.rw` gives a per-project copy of.
#[must_use]
pub fn cache_paths(home: &Path) -> Vec<Cache> {
    vec![
        Cache {
            name: "npm",
            path: under(home, ".npm"),
        },
        Cache {
            name: "nuget",
            path: under(home, ".nuget"),
        },
        Cache {
            name: "cache",
            path: under(home, ".cache"),
        },
    ]
}

/// An agent CLI Willie knows how to run.
pub trait Harness: std::fmt::Debug {
    /// Stable identifier, e.g. `claude-code`.
    fn id(&self) -> &'static str;
    /// Executable name looked up inside the distro.
    fn binary_name(&self) -> &'static str;
    /// Capability matrix.
    fn capabilities(&self) -> HarnessCapabilities;

    /// Layer 1 of the sandbox policy: what a session with this harness
    /// gets before the project has its say. The default is the set that
    /// carries no credential, so a new harness has to ask for
    /// `agent.state` rather than inherit it.
    fn default_capabilities(&self) -> CapabilitySet {
        CapabilitySet {
            project_rw: true,
            agent_state: false,
            tools_ro: true,
            caches_rw: true,
            git_identity: true,
            extra_paths: Vec::new(),
        }
    }

    /// Where this harness keeps its login under `home`, if it has one to
    /// keep. `None` means `agent.state` has nothing to bind: safer than
    /// guessing a directory for a harness that never said.
    fn agent_state(&self, home: &Path) -> Option<AgentState> {
        let _ = home;
        None
    }

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

    /// The layout the image provisions: the CLI's directory and settings
    /// file live under Willie's per-user state, and `~/.claude` plus
    /// `~/.claude.json` are links into it.
    fn agent_state(&self, home: &Path) -> Option<AgentState> {
        const STATE: &str = ".willie/agent-state/claude";
        Some(AgentState {
            dir: under(home, STATE),
            links: vec![
                Link {
                    target: format!("{STATE}/dot-claude"),
                    path: under(home, ".claude"),
                },
                Link {
                    target: format!("{STATE}/claude.json"),
                    path: under(home, ".claude.json"),
                },
            ],
        })
    }

    /// Without its state directory the CLI cannot log in, so the
    /// session would start and be useless. `agent_state` is the only
    /// field that differs from the trait's default; the rest are
    /// repeated here rather than delegated, because `Self` in a unit
    /// struct's expression position names the struct's own value, so
    /// `Harness::default_capabilities(&Self)` would dispatch back to
    /// this override instead of the trait's default body.
    fn default_capabilities(&self) -> CapabilitySet {
        CapabilitySet {
            project_rw: true,
            agent_state: true,
            tools_ro: true,
            caches_rw: true,
            git_identity: true,
            extra_paths: Vec::new(),
        }
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

    /// Claude Code cannot log in without its state directory, so the
    /// one capability that carries a credential is on by default for
    /// it, and the design says so out loud rather than pretending the
    /// session is cheaper than it is.
    #[test]
    fn claude_code_gets_its_login_and_the_ordinary_binds() {
        let set = ClaudeCode.default_capabilities();

        assert!(set.project_rw);
        assert!(set.agent_state);
        assert!(set.tools_ro);
        assert!(set.caches_rw);
        assert!(set.git_identity);
        assert!(set.extra_paths.is_empty());
    }

    /// A harness that overrides nothing: what the trait gives by default.
    #[derive(Debug)]
    struct Quiet;

    impl Harness for Quiet {
        fn id(&self) -> &'static str {
            "quiet"
        }
        fn binary_name(&self) -> &'static str {
            "quiet"
        }
        fn capabilities(&self) -> HarnessCapabilities {
            HarnessCapabilities {
                interactive_tui: false,
                resume: Resume::None,
                headless_stream: false,
            }
        }
        fn installer(&self) -> &'static str {
            "true"
        }
    }

    /// A harness that says nothing gets no credential: a new harness
    /// must ask for the risky bind, never inherit it.
    #[test]
    fn a_harness_that_overrides_nothing_gets_no_credential() {
        let set = Quiet.default_capabilities();

        assert!(set.project_rw);
        assert!(!set.agent_state);
        assert!(set.tools_ro);
    }

    /// The image keeps the login under `~/.willie/agent-state/claude` and
    /// links the two dot paths into it. A sandbox that reproduces the same
    /// links over a private home gives the CLI the login a plain shell in
    /// the distribution sees, and nothing else of the real home.
    #[test]
    fn claude_code_keeps_its_login_under_the_willie_state_dir() {
        let state = ClaudeCode
            .agent_state(Path::new("/home/willie"))
            .expect("Claude Code has a login to bind");

        assert_eq!(
            state.dir,
            PathBuf::from("/home/willie/.willie/agent-state/claude")
        );
        assert_eq!(
            state.links,
            vec![
                Link {
                    target: ".willie/agent-state/claude/dot-claude".into(),
                    path: PathBuf::from("/home/willie/.claude"),
                },
                Link {
                    target: ".willie/agent-state/claude/claude.json".into(),
                    path: PathBuf::from("/home/willie/.claude.json"),
                },
            ]
        );
    }

    /// No layout, no bind: `agent.state` on such a harness grants nothing,
    /// which is safer than guessing a directory.
    #[test]
    fn a_harness_that_says_nothing_has_no_login_to_bind() {
        assert!(Quiet.agent_state(Path::new("/home/willie")).is_none());
    }

    /// Where managed tools land today: the user-local tree the official
    /// installers use, and the .NET root. Bound read-only by `tools.ro`.
    #[test]
    fn the_tool_roots_are_the_user_local_tree_and_dotnet() {
        assert_eq!(
            tool_roots(Path::new("/home/willie")),
            vec![
                PathBuf::from("/home/willie/.local"),
                PathBuf::from("/home/willie/.dotnet"),
            ]
        );
    }

    /// Each package cache has a short name, so a per-project copy can be
    /// kept under it, and the home-relative path tools expect it at.
    #[test]
    fn each_package_cache_has_a_name_and_a_home_relative_target() {
        assert_eq!(
            cache_paths(Path::new("/home/willie")),
            vec![
                Cache {
                    name: "npm",
                    path: PathBuf::from("/home/willie/.npm"),
                },
                Cache {
                    name: "nuget",
                    path: PathBuf::from("/home/willie/.nuget"),
                },
                Cache {
                    name: "cache",
                    path: PathBuf::from("/home/willie/.cache"),
                },
            ]
        );
    }

    /// `willie-core` cannot depend on this crate, so its sandbox guard
    /// names the managed tool roots and package caches independently.
    /// Two lists that must agree can drift; this test is what keeps
    /// them from drifting apart: a sixth cache added here without a
    /// matching entry in the guard fails here, not in production.
    #[test]
    fn every_tool_root_and_cache_path_is_refused_as_an_extra_path() {
        const HOME: &str = "/home/willie";

        for root in tool_roots(Path::new(HOME)) {
            assert!(
                willie_core::sandbox::guard_extra_path(
                    &root.to_string_lossy(),
                    HOME,
                )
                .is_some(),
                "{root:?}"
            );
        }
        for cache in cache_paths(Path::new(HOME)) {
            assert!(
                willie_core::sandbox::guard_extra_path(
                    &cache.path.to_string_lossy(),
                    HOME,
                )
                .is_some(),
                "{cache:?}"
            );
        }
    }
}
