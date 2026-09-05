//! Applying the plan: what must be true before the helper is spawned,
//! what this version applies, and how the helper's exit and the stop
//! ladder map back to the harness. The plan itself is data in
//! `willie_linux::sandbox`; this is the I/O around it.

pub mod inner;

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

/// Point one extra path's bind at `resolved`, leaving its destination
/// as configured. The plan appends one bind per extra path after
/// everything else, so the last op with that destination is that bind.
fn rebind_source(ops: &mut [plan::Op], destination: &str, resolved: &str) {
    for op in ops.iter_mut().rev() {
        if let plan::Op::Bind { src, dest, .. } = op
            && dest == destination
        {
            *src = resolved.to_owned();
            return;
        }
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
    let mut plan =
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
        if resolved != extra.path
            && let Some(reason) =
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
        // The helper mounts what this check resolved, not the path as
        // written: a last component that is a symbolic link can be
        // re-pointed between the two, and a link inside the project is
        // writable by every session on it. The destination stays as the
        // policy configured it.
        rebind_source(&mut plan.ops, &extra.path, &resolved);
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
    // The binary that was verified above is the binary that runs: the
    // vector names where the helper lives, and a test points the
    // supervisor at another one.
    let mut argv = bwrap::argv(&plan);
    if let Some(first) = argv.first_mut() {
        *first = helper.to_string_lossy().into_owned();
    }
    Ok(Prepared {
        argv,
        mechanisms: MECHANISMS.iter().map(|m| (*m).to_owned()).collect(),
    })
}

/// Where the namespace helper lives. `WILLIE_SESS_HELPER_BIN` points
/// the supervisor at another one so a test can drive the refusal path
/// without a kernel that refuses; test-only, like the stop grace and
/// the harness wait.
#[must_use]
pub fn helper_path() -> std::path::PathBuf {
    std::env::var_os("WILLIE_SESS_HELPER_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(bwrap::BWRAP))
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

/// Every pid a `/proc/<pid>/task/<pid>/children` file names, or none if
/// it holds anything that is not a pid.
#[must_use]
pub fn parse_child_pids(text: &str) -> Option<Vec<i32>> {
    text.split_whitespace()
        .map(str::parse::<i32>)
        .collect::<Result<Vec<_>, _>>()
        .ok()
}

/// The one pid such a file names, or none: the helper's monitor has one
/// child (the reaper) and the reaper one child (the harness); any other
/// shape is not what we launched.
#[must_use]
pub fn parse_children(text: &str) -> Option<i32> {
    match parse_child_pids(text)?.as_slice() {
        [only] => Some(*only),
        _ => None,
    }
}

/// Whether such a file still names `harness`. This is what tells a
/// *noisy* shape from a *gone* one: the reaper is pid 1 inside the
/// session's namespace, so an orphaned grandchild is reparented onto it
/// and hides which of its children is the harness, while the harness is
/// still one of them. A file that no longer names it means the harness
/// has exited, and its number is then free for any process in the
/// distribution to take.
#[must_use]
pub fn children_include(text: &str, harness: i32) -> bool {
    parse_child_pids(text).is_some_and(|ids| ids.contains(&harness))
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

/// Whether the harness a session recorded at start-up is still one of
/// the reaper's children, which is the only case where a pid resolved
/// once may be signalled later: the shape is noisy rather than gone
/// (see [`children_include`]). False whenever the reaper cannot be
/// reached or no longer names it — the harness has exited, the number
/// is stale, and signalling it would reach whatever holds it now.
#[cfg(target_os = "linux")]
#[must_use]
pub fn harness_still_behind(
    monitor: libc::pid_t,
    harness: libc::pid_t,
) -> bool {
    let Some(reaper) = only_child(monitor) else {
        return false;
    };
    fs::read_to_string(format!("/proc/{reaper}/task/{reaper}/children"))
        .is_ok_and(|text| children_include(&text, harness))
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
/// How the wait for the harness ended. The two ways it can end without
/// a harness are not the same session: one is a helper that refused
/// while building the namespace, which is a failure with a cause worth
/// recording, and the other is a helper still working past the ceiling,
/// which starts the session with the promise given up.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessWait {
    Running(libc::pid_t),
    /// The helper's monitor is gone and no harness ever appeared.
    HelperGone,
    /// The ceiling passed with the helper still alive.
    GaveUp,
}

/// At most this much of what the helper said reaches the record. The
/// log is append-only and read by people; one runaway helper must not
/// fill a screen of it.
const HELPER_WORDS_LIMIT: usize = 400;

/// How many bytes of the terminal are read back when the helper
/// refused. Generous against the limit above, because the helper may
/// have written control sequences the words are buried in.
pub const HELPER_DRAIN: usize = 16 * 1024;

/// What the helper said, as the one line an event can carry. A terminal
/// turns each newline into a carriage return and a newline, and a helper
/// may complain more than once, so the lines are joined and the empty
/// ones dropped.
fn helper_words(raw: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(raw);
    let joined = text
        .lines()
        .map(|line| line.trim_end_matches('\r').trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    if joined.is_empty() {
        return None;
    }
    if joined.chars().count() > HELPER_WORDS_LIMIT {
        let cut: String = joined.chars().take(HELPER_WORDS_LIMIT).collect();
        return Some(format!("{cut}…"));
    }
    Some(joined)
}

/// The prefix the helper puts on every message it dies with.
const HELPER_PREFIX: &str = "bwrap: ";

/// Whether what came back from the terminal is the helper's own refusal
/// rather than the harness's output.
///
/// Nothing the helper offers says "the namespace was built". It forks
/// the sandboxed child *before* it mounts anything, so a child having
/// existed proves nothing, and a harness that runs and exits inside one
/// poll of the wait leaves exactly the same trace as a helper that never
/// forked one: no grandchild, and a monitor already gone.
///
/// What does tell them apart is that the helper prefixes what it says
/// when it gives up. A harness whose very first line of output were that
/// prefix would be mislabelled; the cost is one wrong word in one event,
/// and the session is recorded either way.
#[must_use]
pub fn is_helper_refusal(raw: &[u8]) -> bool {
    String::from_utf8_lossy(raw)
        .lines()
        .map(|line| line.trim_end_matches('\r').trim_start())
        .find(|line| !line.is_empty())
        .is_some_and(|line| line.starts_with(HELPER_PREFIX))
}

/// The message for a helper that refused after it was executed.
///
/// Everything the supervisor can see before the helper runs is already
/// a coded refusal (`prepare`). What is left is the helper refusing
/// while it builds the namespace, and its only channel is the session's
/// terminal, which nothing is attached to yet. Carrying its words here
/// is the difference between a session that says why it could not start
/// and one that merely appears and disappears.
#[must_use]
pub fn apply_failure(
    raw: &[u8],
    code: Option<i32>,
    signal: Option<i32>,
) -> String {
    match (helper_words(raw), code, signal) {
        (Some(words), _, _) => {
            format!("the namespace helper refused: {words}")
        }
        (None, Some(code), _) => format!(
            "the namespace helper exited with {code} before the harness \
             started, and said nothing"
        ),
        (None, None, Some(signal)) => format!(
            "the namespace helper was killed by signal {signal} before the \
             harness started"
        ),
        (None, None, None) => "the namespace helper ended before the harness \
             started, and said nothing"
            .to_owned(),
    }
}

/// Waits for the harness the helper forks, so that "ready" and
/// "started" both mean a running harness, as they did before the helper
/// stood between the supervisor and it.
#[cfg(target_os = "linux")]
#[must_use]
pub fn wait_for_harness(monitor: libc::pid_t, within: Duration) -> HarnessWait {
    let until = Instant::now() + within;
    loop {
        // The deadline is tested first, so a ceiling of zero always
        // gives up however fast the helper is: a test that asks for no
        // wait at all must get the branch it asked for, not a race.
        if Instant::now() >= until {
            // The session starts anyway, so say that the promise the
            // wait exists to keep has just been given up: the ladder
            // will have to find the harness itself, and may not.
            eprintln!(
                "willie-sess: no harness appeared behind the helper \
                 within {within:?}; starting anyway"
            );
            return HarnessWait::GaveUp;
        }
        if let Some(pid) = harness_pid(monitor) {
            return HarnessWait::Running(pid);
        }
        if !can_still_fork(monitor) {
            return HarnessWait::HelperGone;
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

    /// The guard resolves the path, so the mount must use what it
    /// resolved: a last component that is a symbolic link can be
    /// re-pointed between the check and the mount, and a link inside
    /// the project is writable by every session on it.
    #[cfg(unix)]
    #[test]
    fn an_extra_path_is_bound_from_the_source_the_check_resolved() {
        use willie_core::sandbox::{ExtraPath, PathMode};

        let root = scratch("resolvedsrc");
        let helper = root.join("bwrap");
        touch(&helper);
        let bin = root.join("claude");
        touch(&bin);
        let real = root.join("real");
        fs::create_dir_all(&real).unwrap();
        let link = root.join("shared");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let configured = link.to_string_lossy().into_owned();
        let mut spec =
            spec_under(&root, &bin.to_string_lossy(), &root.to_string_lossy());
        spec.capabilities.extra_paths = vec![ExtraPath {
            path: configured.clone(),
            mode: PathMode::Ro,
        }];

        let prepared = prepare(&spec, &helper).expect("prepared");

        let bind = prepared
            .argv
            .windows(3)
            .find(|w| w[0] == "--ro-bind" && w[2] == configured)
            .expect("the extra path's bind");
        assert_eq!(bind[1], fs::canonicalize(&real).unwrap().to_string_lossy());
        assert_ne!(bind[1], configured);
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
    /// keep, and the one that still starts the session.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_harness_that_never_appears_ends_the_wait_at_the_ceiling() {
        let me = libc::pid_t::try_from(std::process::id()).unwrap();
        let started = Instant::now();

        let waited = wait_for_harness(me, Duration::from_millis(50));

        assert_eq!(waited, HarnessWait::GaveUp);
        assert!(started.elapsed() >= Duration::from_millis(50));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    /// The helper prefixes what it says when it gives up, and that is
    /// the only thing separating its refusal from a harness that ran
    /// and exited before the wait could see it. Both leave a monitor
    /// already gone and no grandchild.
    #[test]
    fn only_the_helper_s_own_prefix_marks_a_refusal() {
        assert!(is_helper_refusal(
            b"bwrap: Can't mount on symlink destination /x\r\n"
        ));
        assert!(is_helper_refusal(b"\r\n  bwrap: No permitted\r\n"));

        assert!(!is_helper_refusal(b""));
        assert!(!is_helper_refusal(b"\r\n\r\n"));
        assert!(!is_helper_refusal(b"hello from the harness\r\n"));
        // The prefix only counts as the first thing said: a harness
        // that quotes the helper later has not refused anything.
        assert!(!is_helper_refusal(b"building\r\nbwrap: quoted\r\n"));
    }

    /// The helper's own complaint goes to the terminal, which is the
    /// session's only output channel and which nothing is attached to
    /// when it refuses while building the namespace. Carrying it into
    /// the record is the difference between a session that says why it
    /// could not start and one that merely disappears.
    #[test]
    fn a_refusal_carries_the_helper_s_own_words() {
        let raw = b"bwrap: Can't mount on symlink destination /home/w/.local/bin/claude\r\n";

        let text = apply_failure(raw, Some(1), None);

        assert!(
            text.contains("Can't mount on symlink destination"),
            "{text}"
        );
        assert!(text.contains("/home/w/.local/bin/claude"), "{text}");
        assert!(!text.contains('\r'), "{text}");
        assert!(!text.contains('\n'), "{text}");
    }

    /// A terminal turns every newline into a carriage return and a
    /// newline, and a helper may say several things. The record takes
    /// one line, so the lines are joined and the blank ones dropped.
    #[test]
    fn several_lines_become_one_and_blank_ones_are_dropped() {
        let raw = b"bwrap: first\r\n\r\nbwrap: second\r\n";

        let text = apply_failure(raw, Some(1), None);

        assert!(text.contains("bwrap: first; bwrap: second"), "{text}");
    }

    /// A helper that says nothing at all still has to produce a record
    /// someone can act on, so the exit stands in for the words.
    #[test]
    fn a_silent_refusal_is_reported_by_its_exit() {
        let text = apply_failure(b"", Some(1), None);

        assert!(text.contains('1'), "{text}");
        assert!(text.contains("before the harness"), "{text}");

        let killed = apply_failure(b"   \r\n", None, Some(9));

        assert!(killed.contains('9'), "{killed}");
    }

    /// An event log is append-only and read by people, so one runaway
    /// helper must not put a screenful into it.
    #[test]
    fn a_helper_that_will_not_stop_talking_is_cut_short() {
        let raw = "bwrap: ".repeat(400);

        let text = apply_failure(raw.as_bytes(), Some(1), None);

        assert!(text.len() < 600, "{}", text.len());
        assert!(text.ends_with('…'), "{text}");
    }

    /// The path that was checked is the path that runs: `prepare`
    /// verifies a helper and the vector must then execute that one, or
    /// the check answers for a different binary than the launch.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_vector_runs_the_helper_that_was_verified() {
        let root = scratch("helper-argv");
        let helper = root.join("bwrap");
        touch(&helper);
        let bin = root.join("claude");
        touch(&bin);
        let spec =
            spec_under(&root, &bin.to_string_lossy(), &root.to_string_lossy());

        let prepared = prepare(&spec, &helper).expect("prepared");

        assert_eq!(prepared.argv[0], helper.to_string_lossy());
        let _ = fs::remove_dir_all(&root);
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

    /// The stop ladder may fall back on the pid it resolved at start-up
    /// only while the shape is noisy — the reaper collected an orphan
    /// and has more than one child — never once the harness is gone,
    /// when the number is free for anything in the distribution to
    /// take.
    #[test]
    fn a_cached_harness_counts_only_while_the_children_file_still_names_it() {
        assert!(children_include("41 42 43\n", 42));
        assert!(children_include("42\n", 42));
        assert!(!children_include("41 43\n", 42));
        assert!(!children_include("", 42));
        assert!(!children_include("42 x", 42));
        assert_eq!(parse_child_pids("41 42"), Some(vec![41, 42]));
        assert_eq!(parse_child_pids("41 x"), None);
    }
}
