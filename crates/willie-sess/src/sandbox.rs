//! Applying the plan: what must be true before the helper is spawned,
//! what this version applies, and how the helper's exit and the stop
//! ladder map back to the harness. The plan itself is data in
//! `willie_linux::sandbox`; this is the I/O around it.

use std::{fmt, fs, io, path::Path};
#[cfg(target_os = "linux")]
use std::{
    thread,
    time::{Duration, Instant},
};

use willie_core::session::SessionSpec;
use willie_linux::sandbox::{self as plan, bwrap};

/// What this version applies, named in the `sandbox_applied` event. The
/// syscall filter, the limits and path-based restriction join the list
/// when they land, and a session that ran with less says so forever.
pub const MECHANISMS: [&str; 2] = ["namespaces", "mounts"];

/// Everything the supervisor needs to spawn the confined session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub argv: Vec<String>,
    pub mechanisms: Vec<String>,
}

#[derive(Debug)]
pub enum PrepareError {
    /// The spec names a harness this build does not know.
    HarnessUnknown(String),
    Plan(plan::PlanError),
    /// The namespace helper is not in the image.
    BackendMissing(String),
    /// A per-project cache directory could not be created.
    CacheDir {
        path: String,
        error: io::Error,
    },
    /// The harness binary or the workspace is not there. Checked here
    /// because inside the helper the same failure is a bare exit 1.
    Harness {
        step: &'static str,
        error: io::Error,
        path: String,
    },
    /// An extra path the policy's guard allowed as written, but which
    /// resolves into a guarded location, or which cannot be resolved at
    /// all. `willie-core` does no I/O and can see neither; here both are
    /// checked before anything is mounted.
    ExtraPath {
        path: String,
        detail: String,
    },
    /// A path the plan binds without tolerance is not on this machine.
    /// Checked here because inside the helper the same failure is a
    /// message on the session's terminal and a bare exit 1, which no
    /// log keeps.
    BindSource {
        path: String,
    },
}

impl PrepareError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::HarnessUnknown(_) | Self::Plan(_) => "spec_invalid",
            Self::BackendMissing(_) => "sandbox_backend_missing",
            Self::CacheDir { .. } | Self::BindSource { .. } => {
                "sandbox_apply_failed"
            }
            Self::Harness { .. } => "harness_exec_failed",
            Self::ExtraPath { .. } => "sandbox_profile_invalid",
        }
    }
}

impl fmt::Display for PrepareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HarnessUnknown(id) => write!(f, "unknown harness `{id}`"),
            Self::Plan(e) => write!(f, "{e}"),
            Self::BackendMissing(path) => {
                write!(f, "the namespace helper is missing: {path}")
            }
            Self::CacheDir { path, error } => {
                write!(f, "cannot create the cache directory {path}: {error}")
            }
            Self::Harness { step, error, path } => {
                write!(f, "{step}: {error} ({path})")
            }
            Self::ExtraPath { path, detail } => write!(f, "`{path}`: {detail}"),
            Self::BindSource { path } => {
                write!(f, "the sandbox needs {path}, which is not there")
            }
        }
    }
}

impl std::error::Error for PrepareError {}

fn not_found() -> io::Error {
    io::Error::from(io::ErrorKind::NotFound)
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Resolve the plan and check what the helper would otherwise report as
/// a bare exit: the helper exists, the harness binary is executable,
/// the workspace is a directory, and every source the plan binds
/// without tolerance is on this machine. The per-project caches are
/// created first, because the plan both creates and binds those — a
/// session never starts without them. `helper` is injected so the
/// checks are testable without the image.
pub fn prepare(
    spec: &SessionSpec,
    helper: &Path,
) -> Result<Prepared, PrepareError> {
    let harness = willie_harness::registry()
        .into_iter()
        .find(|h| h.id() == spec.harness)
        .ok_or_else(|| PrepareError::HarnessUnknown(spec.harness.clone()))?;
    let plan =
        plan::plan(spec, harness.as_ref()).map_err(PrepareError::Plan)?;
    if !helper.is_file() {
        return Err(PrepareError::BackendMissing(
            helper.to_string_lossy().into_owned(),
        ));
    }
    let binary = plan.argv.first().cloned().unwrap_or_default();
    if !is_executable_file(Path::new(&binary)) {
        return Err(PrepareError::Harness {
            step: "cannot execute the harness",
            error: not_found(),
            path: binary,
        });
    }
    if !Path::new(&plan.workspace).is_dir() {
        return Err(PrepareError::Harness {
            step: "cannot enter the workspace",
            error: not_found(),
            path: plan.workspace.clone(),
        });
    }
    // The policy's guard is lexical, because `willie-core` does no I/O:
    // an extra path that is a symbolic link into a guarded location
    // passes it as written. Here the resolved path is available, so the
    // same guard runs again on what will actually be mounted.
    for extra in &spec.capabilities.extra_paths {
        let resolved = fs::canonicalize(&extra.path).map_err(|e| {
            PrepareError::ExtraPath {
                path: extra.path.clone(),
                detail: format!("cannot be resolved: {e}"),
            }
        })?;
        let resolved = resolved.to_string_lossy().into_owned();
        if resolved == extra.path {
            continue;
        }
        if let Some(reason) =
            willie_core::sandbox::guard_extra_path(&resolved, &plan.home)
        {
            return Err(PrepareError::ExtraPath {
                path: extra.path.clone(),
                detail: format!(
                    "resolves to `{resolved}`, which cannot be an extra \
                     path: {reason}"
                ),
            });
        }
    }
    // The caches first: the plan both creates and binds those, so they
    // have to exist before the sources are checked.
    for dir in &plan.ensure_dirs {
        fs::create_dir_all(dir).map_err(|error| PrepareError::CacheDir {
            path: dir.clone(),
            error,
        })?;
    }
    // Every remaining source the plan binds without tolerance. A missing
    // one kills the helper with a message that goes to the session's
    // terminal and nowhere else, so the whole class is a coded refusal
    // here instead. A tolerant bind is skipped: that is what tolerant
    // means — the layout is fixed, the machine is not.
    for op in &plan.ops {
        if let plan::Op::Bind {
            src,
            optional: false,
            ..
        } = op
            && !Path::new(src).exists()
        {
            return Err(PrepareError::BindSource { path: src.clone() });
        }
    }
    Ok(Prepared {
        argv: bwrap::argv(&plan),
        mechanisms: MECHANISMS.iter().map(|m| (*m).to_owned()).collect(),
    })
}

/// The helper reports a harness killed by signal `n` as exit `128 + n`
/// (decision 0016). Put the signal back so the record says what
/// happened; a harness that itself exits in that range is recorded as
/// a signal death, the shell's own convention.
#[must_use]
pub fn helper_exit(
    code: Option<i32>,
    signal: Option<i32>,
) -> (Option<i32>, Option<i32>) {
    match (code, signal) {
        (Some(c), None) if (129..=192).contains(&c) => (None, Some(c - 128)),
        other => other,
    }
}

/// The state character in a `/proc/<pid>/stat` line: the field after
/// the command, which is parenthesised and may itself hold spaces and
/// parentheses, so it is read after the last `)`.
#[must_use]
pub fn parse_state(stat: &str) -> Option<char> {
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .next()?
        .chars()
        .next()
}

/// The one pid a `/proc/<pid>/task/<pid>/children` file names, or none:
/// the helper's monitor has one child (the reaper) and the reaper one
/// child (the harness); any other shape is not what we launched.
#[must_use]
pub fn parse_children(text: &str) -> Option<i32> {
    let mut ids = text.split_whitespace().map(str::parse::<i32>);
    let first = ids.next()?.ok()?;
    if ids.next().is_some() {
        return None;
    }
    Some(first)
}

/// The harness process behind the helper's monitor, resolved when a
/// signal has to reach the harness and not the monitor (decision
/// 0016: the monitor dies on the polite signals and takes the session
/// with it). `None` when the shape is gone: the caller then signals
/// the group, which ends the session — the closed direction.
#[cfg(target_os = "linux")]
#[must_use]
pub fn harness_pid(monitor: libc::pid_t) -> Option<libc::pid_t> {
    let reaper = only_child(monitor)?;
    only_child(reaper)
}

/// How long the supervisor waits for the helper to build the namespace
/// and fork the harness before answering ready. Measured at about eight
/// milliseconds; the ceiling is generous because the daemon allows ten
/// seconds for the readiness line, and a helper that dies during setup
/// is noticed long before it expires.
#[cfg(target_os = "linux")]
pub const HARNESS_WAIT: Duration = Duration::from_secs(2);

#[cfg(target_os = "linux")]
const HARNESS_POLL: Duration = Duration::from_millis(2);

/// The ceiling on that wait. `WILLIE_SESS_HARNESS_WAIT_MS` shortens it
/// for the tests, the way `WILLIE_SESS_STOP_GRACE_MS` shortens the stop
/// ladder; the default is `HARNESS_WAIT`.
#[cfg(target_os = "linux")]
#[must_use]
pub fn harness_wait() -> Duration {
    std::env::var("WILLIE_SESS_HARNESS_WAIT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map_or(HARNESS_WAIT, Duration::from_millis)
}

/// The harness process, waited for. The helper is executed some
/// milliseconds before it unshares and forks, so right after the spawn
/// there is nothing behind the monitor yet, and a stop arriving in that
/// window would find no harness and signal the group — which ends the
/// session instead of asking it to. Waiting here closes the window:
/// afterwards the readiness line and the `started` event both mean the
/// harness is running, as they did before the helper stood between them.
///
/// `None` when the monitor is already gone or a zombie (the helper
/// refused, and there is nothing to wait for) or when the deadline
/// passes; the session starts anyway and the ladder re-resolves.
#[cfg(target_os = "linux")]
#[must_use]
pub fn wait_for_harness(
    monitor: libc::pid_t,
    within: Duration,
) -> Option<libc::pid_t> {
    let until = Instant::now() + within;
    loop {
        if let Some(pid) = harness_pid(monitor) {
            return Some(pid);
        }
        if !can_still_fork(monitor) {
            return None;
        }
        if Instant::now() >= until {
            // The session starts anyway, so say that the promise the
            // wait exists to keep has just been given up: the ladder
            // will have to find the harness itself, and may not.
            eprintln!(
                "willie-sess: no harness appeared behind the helper \
                 within {within:?}; starting anyway"
            );
            return None;
        }
        thread::sleep(HARNESS_POLL);
    }
}

/// Whether the helper's monitor is still a process that could fork the
/// harness: its entry is readable and it is not a zombie.
#[cfg(target_os = "linux")]
fn can_still_fork(pid: libc::pid_t) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| parse_state(&stat))
        .is_some_and(|state| state != 'Z')
}

#[cfg(target_os = "linux")]
fn only_child(pid: libc::pid_t) -> Option<libc::pid_t> {
    let text =
        fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")).ok()?;
    parse_children(&text)
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs, path::PathBuf};

    use willie_core::{
        id::{ProjectId, SessionId},
        sandbox::CapabilitySet,
        session::SessionSpec,
    };

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("willie-sess-sandbox-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        fs::write(path, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755))
                .unwrap();
        }
    }

    /// A spec whose paths all live under `root`, with the per-project
    /// caches on so `prepare` has directories to create.
    fn spec_under(root: &Path, binary: &str, workspace: &str) -> SessionSpec {
        let mut env = BTreeMap::new();
        env.insert("HOME".to_owned(), root.to_string_lossy().into_owned());
        SessionSpec {
            id: SessionId::new(),
            project_id: ProjectId::new(),
            harness: "claude-code".into(),
            workspace: workspace.to_owned(),
            socket: root.join("s.sock").to_string_lossy().into_owned(),
            argv: vec![binary.to_owned()],
            env,
            created_at: "1".into(),
            willie_version: "0".into(),
            resumed_from: None,
            capabilities: CapabilitySet {
                caches_rw: true,
                ..CapabilitySet::default()
            },
        }
    }

    #[test]
    fn prepare_creates_the_per_project_caches_and_renders_the_vector() {
        let root = scratch("ok");
        let helper = root.join("bwrap");
        touch(&helper);
        let bin = root.join("claude");
        touch(&bin);
        let ws = root.join("ws");
        fs::create_dir_all(&ws).unwrap();
        let spec =
            spec_under(&root, &bin.to_string_lossy(), &ws.to_string_lossy());

        let prepared = prepare(&spec, &helper).expect("prepared");

        assert_eq!(prepared.argv[0], bwrap::BWRAP);
        assert_eq!(prepared.mechanisms, vec!["namespaces", "mounts"]);
        let caches = root
            .join(".willie")
            .join("caches")
            .join(spec.project_id.to_string());
        for name in ["npm", "nuget", "cache"] {
            assert!(caches.join(name).is_dir(), "{name}");
        }
        assert_eq!(prepared.argv.last().unwrap(), &bin.to_string_lossy());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_helper_is_sandbox_backend_missing_before_anything_is_created()
    {
        let root = scratch("nohelper");
        let bin = root.join("claude");
        touch(&bin);
        let spec =
            spec_under(&root, &bin.to_string_lossy(), &root.to_string_lossy());

        let err = prepare(&spec, &root.join("absent")).unwrap_err();

        assert_eq!(err.code(), "sandbox_backend_missing");
        assert!(!root.join(".willie").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_harness_binary_is_harness_exec_failed_naming_it() {
        let root = scratch("nobin");
        let helper = root.join("bwrap");
        touch(&helper);
        let spec =
            spec_under(&root, "/nonexistent/claude", &root.to_string_lossy());

        let err = prepare(&spec, &helper).unwrap_err();

        assert_eq!(err.code(), "harness_exec_failed");
        assert!(
            err.to_string().starts_with("cannot execute the harness"),
            "{err}"
        );
        assert!(err.to_string().contains("/nonexistent/claude"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_workspace_is_harness_exec_failed_naming_the_directory() {
        let root = scratch("nows");
        let helper = root.join("bwrap");
        touch(&helper);
        let bin = root.join("claude");
        touch(&bin);
        let gone = root.join("gone");
        let spec =
            spec_under(&root, &bin.to_string_lossy(), &gone.to_string_lossy());

        let err = prepare(&spec, &helper).unwrap_err();

        assert_eq!(err.code(), "harness_exec_failed");
        assert!(
            err.to_string().starts_with("cannot enter the workspace"),
            "{err}"
        );
        assert!(err.to_string().contains("gone"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    /// The policy's guard is lexical, because `willie-core` does no I/O,
    /// so a symbolic link into a guarded location passes it as written.
    /// The supervisor sees where it goes and refuses what would actually
    /// be mounted.
    #[cfg(unix)]
    #[test]
    fn an_extra_path_that_resolves_into_a_guarded_location_is_refused() {
        use willie_core::sandbox::{ExtraPath, PathMode};

        let root = scratch("symlink");
        let helper = root.join("bwrap");
        touch(&helper);
        let bin = root.join("claude");
        touch(&bin);
        let link = root.join("shared");
        std::os::unix::fs::symlink("/etc", &link).unwrap();
        let mut spec =
            spec_under(&root, &bin.to_string_lossy(), &root.to_string_lossy());
        spec.capabilities.extra_paths = vec![ExtraPath {
            path: link.to_string_lossy().into_owned(),
            mode: PathMode::Ro,
        }];

        let err = prepare(&spec, &helper).unwrap_err();

        assert_eq!(err.code(), "sandbox_profile_invalid");
        assert!(err.to_string().contains("/etc"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    /// An extra path that is not there refuses the session here, with a
    /// code, rather than letting the helper fail with its own message
    /// from inside a namespace nobody is watching.
    #[test]
    fn an_extra_path_that_does_not_exist_is_refused() {
        use willie_core::sandbox::{ExtraPath, PathMode};

        let root = scratch("noextra");
        let helper = root.join("bwrap");
        touch(&helper);
        let bin = root.join("claude");
        touch(&bin);
        let mut spec =
            spec_under(&root, &bin.to_string_lossy(), &root.to_string_lossy());
        spec.capabilities.extra_paths = vec![ExtraPath {
            path: root.join("absent").to_string_lossy().into_owned(),
            mode: PathMode::Ro,
        }];

        let err = prepare(&spec, &helper).unwrap_err();

        assert_eq!(err.code(), "sandbox_profile_invalid");
        assert!(err.to_string().contains("cannot be resolved"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unknown_harness_id_is_spec_invalid() {
        let root = scratch("unknown");
        let helper = root.join("bwrap");
        touch(&helper);
        let mut spec = spec_under(&root, "/x", &root.to_string_lossy());
        spec.harness = "not-a-harness".into();

        let err = prepare(&spec, &helper).unwrap_err();

        assert_eq!(err.code(), "spec_invalid");
        assert!(err.to_string().contains("not-a-harness"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_spec_without_a_home_is_spec_invalid() {
        let root = scratch("nohome");
        let helper = root.join("bwrap");
        touch(&helper);
        let mut spec = spec_under(&root, "/x", &root.to_string_lossy());
        spec.env.remove("HOME");

        assert_eq!(prepare(&spec, &helper).unwrap_err().code(), "spec_invalid");
        let _ = fs::remove_dir_all(&root);
    }

    /// The helper reports a harness killed by signal n as exit 128 + n;
    /// the record should say the signal, as it did before the helper.
    #[test]
    fn a_helper_exit_above_128_is_the_harness_signal() {
        assert_eq!(helper_exit(Some(143), None), (None, Some(15)));
        assert_eq!(helper_exit(Some(130), None), (None, Some(2)));
        assert_eq!(helper_exit(Some(137), None), (None, Some(9)));
    }

    #[test]
    fn an_ordinary_exit_or_a_direct_signal_passes_through() {
        assert_eq!(helper_exit(Some(0), None), (Some(0), None));
        assert_eq!(helper_exit(Some(7), None), (Some(7), None));
        assert_eq!(helper_exit(Some(128), None), (Some(128), None));
        assert_eq!(helper_exit(Some(193), None), (Some(193), None));
        assert_eq!(helper_exit(None, Some(9)), (None, Some(9)));
    }

    /// The helper dies with a message on the session's terminal and a
    /// bare exit when a source it must bind is not there, so the whole
    /// class is refused here, by name, before any process exists.
    #[test]
    fn a_missing_non_tolerant_bind_source_is_sandbox_apply_failed_naming_it() {
        let root = scratch("nobind");
        let helper = root.join("bwrap");
        touch(&helper);
        let bin = root.join("claude");
        touch(&bin);
        let mut spec =
            spec_under(&root, &bin.to_string_lossy(), &root.to_string_lossy());
        spec.capabilities.git_identity = true;

        let err = prepare(&spec, &helper).unwrap_err();

        assert_eq!(err.code(), "sandbox_apply_failed");
        assert!(err.to_string().contains(".gitconfig"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    /// A tool root the machine has not installed is bound tolerantly, so
    /// its absence is not a refusal: the layout is fixed, the machine is
    /// not.
    #[test]
    fn a_missing_tolerant_bind_source_is_not_a_refusal() {
        let root = scratch("tolerant");
        let helper = root.join("bwrap");
        touch(&helper);
        let bin = root.join("claude");
        touch(&bin);
        let mut spec =
            spec_under(&root, &bin.to_string_lossy(), &root.to_string_lossy());
        spec.capabilities.tools_ro = true;

        assert!(prepare(&spec, &helper).is_ok());
        let _ = fs::remove_dir_all(&root);
    }

    /// The command in `/proc/<pid>/stat` is parenthesised and may hold
    /// spaces and parentheses of its own, so the state is the first
    /// field after the last `)`.
    #[test]
    fn the_state_is_read_after_the_command_however_it_is_named() {
        assert_eq!(parse_state("42 (bwrap) S 1 0 0"), Some('S'));
        assert_eq!(parse_state("42 (odd )name) Z 1 0"), Some('Z'));
        assert_eq!(parse_state("42 (bwrap)"), None);
        assert_eq!(parse_state("nonsense"), None);
    }

    /// The ceiling exists so a helper that never forks cannot hold a
    /// session open for ever. This process is alive and has nothing
    /// behind it shaped like a helper, so the wait can only end by its
    /// deadline — the one branch that gives up the promise it exists to
    /// keep.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_harness_that_never_appears_ends_the_wait_at_the_ceiling() {
        let me = libc::pid_t::try_from(std::process::id()).unwrap();
        let started = Instant::now();

        let found = wait_for_harness(me, Duration::from_millis(50));

        assert_eq!(found, None);
        assert!(started.elapsed() >= Duration::from_millis(50));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    /// The helper's monitor has one child, the reaper; the reaper has one
    /// child, the harness. Anything else is not the shape we launched.
    #[test]
    fn the_children_file_yields_exactly_one_pid_or_nothing() {
        assert_eq!(parse_children("4242 \n"), Some(4242));
        assert_eq!(parse_children("4242"), Some(4242));
        assert_eq!(parse_children(""), None);
        assert_eq!(parse_children("1 2 "), None);
        assert_eq!(parse_children("x"), None);
    }
}
