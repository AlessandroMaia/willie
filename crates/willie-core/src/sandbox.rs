//! The sandbox policy: what a session is allowed to reach.
//!
//! Pure data and one merge. The daemon resolves a project's policy once
//! per session and writes it into the immutable spec; the supervisor
//! applies what it is given. Nothing here knows how a capability
//! becomes a mount, and nothing here does I/O.
//!
//! Not `willie_harness::HarnessCapabilities`, which says what a harness
//! binary can do. This says what a session may touch.

use serde::{Deserialize, Serialize};

/// One named thing a session may reach. Names grant; there is no
/// capability that denies.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    ProjectRw,
    AgentState,
    ToolsRo,
    CachesRw,
    GitIdentity,
    ExtraPaths,
    HomePersistent,
    Ssh,
    MntAll,
    WindowsInterop,
}

impl Capability {
    /// Every capability, in the order the UI lists them: what ships
    /// first, then what is deferred.
    pub const ALL: [Self; 10] = [
        Self::ProjectRw,
        Self::AgentState,
        Self::ToolsRo,
        Self::CachesRw,
        Self::GitIdentity,
        Self::ExtraPaths,
        Self::HomePersistent,
        Self::Ssh,
        Self::MntAll,
        Self::WindowsInterop,
    ];

    /// The documented dotted name (`docs/ARCHITECTURE.md` §3.3), shown
    /// by the UI and by `sandbox explain`. The profile file uses the
    /// snake_case field names instead, because a dotted TOML key would
    /// nest a table; this is the one place the two meet.
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            Self::ProjectRw => "project.rw",
            Self::AgentState => "agent.state",
            Self::ToolsRo => "tools.ro",
            Self::CachesRw => "caches.rw",
            Self::GitIdentity => "git.identity",
            Self::ExtraPaths => "extra.paths",
            Self::HomePersistent => "home.persistent",
            Self::Ssh => "ssh",
            Self::MntAll => "mnt.all",
            Self::WindowsInterop => "windows.interop",
        }
    }

    /// The one sentence shown beside the switch. It says what enabling
    /// costs, because that is the decision; what the capability is
    /// called is already on screen.
    #[must_use]
    pub fn consequence(self) -> &'static str {
        match self {
            Self::ProjectRw => {
                "the session edits the project, which is why it exists"
            }
            Self::AgentState => {
                "anything the agent runs can use the harness's login"
            }
            Self::ToolsRo => {
                "the agent runs the managed tools and cannot change them"
            }
            Self::CachesRw => {
                "package downloads survive between this project's sessions"
            }
            Self::GitIdentity => "commits carry the user's name and address",
            Self::ExtraPaths => {
                "each entry reaches outside the project, and is recorded \
                 in the session"
            }
            Self::HomePersistent => {
                "the home directory survives the session instead of being \
                 discarded with it"
            }
            Self::Ssh => {
                "the agent can use the user's ssh keys and agent socket"
            }
            Self::MntAll => "the agent reaches every Windows drive",
            Self::WindowsInterop => {
                "the agent runs Windows programs: towards Windows this is \
                 the same as no sandbox"
            }
        }
    }

    /// Whether this Willie can apply it. The four that cannot stay in
    /// the enum so a profile written for a later version is refused by
    /// name instead of being silently ignored.
    #[must_use]
    pub fn is_implemented(self) -> bool {
        matches!(
            self,
            Self::ProjectRw
                | Self::AgentState
                | Self::ToolsRo
                | Self::CachesRw
                | Self::GitIdentity
                | Self::ExtraPaths
        )
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum PathMode {
    Ro,
    Rw,
}

/// One path bound into the session beyond the project itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtraPath {
    /// Absolute, inside the distribution. Checked by `resolve`.
    pub path: String,
    pub mode: PathMode,
}

/// The policy a session runs with: the result of the merge, and what
/// the spec carries. `project_rw` is always true and is kept as a field
/// so `sandbox explain` can list it beside the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySet {
    pub project_rw: bool,
    pub agent_state: bool,
    pub tools_ro: bool,
    pub caches_rw: bool,
    pub git_identity: bool,
    #[serde(default)]
    pub extra_paths: Vec<ExtraPath>,
}

impl CapabilitySet {
    /// Whether this set grants `capability`. The four deferred names
    /// have no field, because a set can never grant what this version
    /// cannot apply; `extra.paths` is granted once the list holds an
    /// entry. The one place the enum and the record meet, so nothing
    /// downstream restates the mapping.
    #[must_use]
    pub fn enabled(&self, capability: Capability) -> bool {
        match capability {
            Capability::ProjectRw => self.project_rw,
            Capability::AgentState => self.agent_state,
            Capability::ToolsRo => self.tools_ro,
            Capability::CachesRw => self.caches_rw,
            Capability::GitIdentity => self.git_identity,
            Capability::ExtraPaths => !self.extra_paths.is_empty(),
            Capability::HomePersistent
            | Capability::Ssh
            | Capability::MntAll
            | Capability::WindowsInterop => false,
        }
    }
}

/// What a spec written before sandboxing resolves to. It does not mean
/// "unconfined": such a session runs inside the boundary like any
/// other and reaches a private home, its workspace and the harness
/// binary, and nothing else of the machine. What it must never mean is
/// "reach nothing" — the project bind is what makes a session a
/// session, so it is on even here.
impl Default for CapabilitySet {
    fn default() -> Self {
        Self {
            project_rw: true,
            agent_state: false,
            tools_ro: false,
            caches_rw: false,
            git_identity: false,
            extra_paths: Vec::new(),
        }
    }
}

/// Layer 2: what one project changes about its harness's defaults.
/// `None` means "whatever the harness decided". The four deferred
/// capabilities are accepted here so that enabling one can be refused
/// with its name; disabling one is already true and costs nothing.
/// Every field is skipped when absent, because TOML has no null: a
/// profile that says nothing has to serialise to a table that says
/// nothing, not to ten keys the format cannot express.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SandboxProfile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_rw: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_state: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools_ro: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caches_rw: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_identity: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extra_paths: Vec<ExtraPath>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home_persistent: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mnt_all: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub windows_interop: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CapabilityError {
    #[error("`{}` is not implemented in this version", .0.display_name())]
    Unsupported(Capability),
    #[error("`{path}` is not an absolute path")]
    ExtraPathNotAbsolute { path: String },
    #[error("`{path}` cannot be an extra path: {reason}")]
    ExtraPathGuarded { path: String, reason: &'static str },
}

impl CapabilityError {
    /// Stable machine-readable code (see docs/PROTOCOL.md).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unsupported(_) => "sandbox_capability_unsupported",
            Self::ExtraPathNotAbsolute { .. } => "sandbox_profile_invalid",
            Self::ExtraPathGuarded { .. } => "sandbox_profile_invalid",
        }
    }

    #[must_use]
    pub fn remediation(&self) -> String {
        match self {
            Self::Unsupported(c) => format!(
                "remove `{}` from the project's sandbox settings; this \
                 version cannot apply it",
                c.display_name()
            ),
            Self::ExtraPathNotAbsolute { .. } => {
                "give an absolute path as it appears inside the \
                 distribution, starting with `/`"
                    .to_owned()
            }
            Self::ExtraPathGuarded { reason, .. } => {
                format!("this version will not grant it: {reason}")
            }
        }
    }
}

/// True if any `/`-separated component of `path` is `..`. Checked
/// before anything else, and refused unconditionally: resolving `..`
/// lexically would be wrong wherever a symbolic link sits along the
/// way, and refusing costs a caller nothing, because the same location
/// is always nameable plainly.
fn has_dotdot_component(path: &str) -> bool {
    path.split('/').any(|c| c == "..")
}

/// Collapses repeated separators and drops `.` components and a
/// trailing separator, without resolving `..`. Assumes `path` has
/// already been checked for a `..` component; called on `home` too, so
/// a `WILLIE_HOME` with a trailing separator still compares correctly.
fn normalize_lexical(path: &str) -> String {
    let mut out = String::from("/");
    for part in path.split('/').filter(|p| !p.is_empty() && *p != ".") {
        if out.len() > 1 {
            out.push('/');
        }
        out.push_str(part);
    }
    out
}

/// Whether `path` is `prefix` itself or nested under it at a
/// `/`-separated component boundary, never merely sharing characters
/// (`/mnt` must not match `/mntx`).
fn is_prefix_path(prefix: &str, path: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// How a guarded location is refused. Both directions matter: a path
/// under a guarded location reaches it piecemeal, and a path above a
/// guarded location contains it, so granting the ancestor is the same
/// as granting the location itself.
#[derive(Clone, Copy)]
enum Flavor {
    /// Refuses the location, everything under it, and everything
    /// above it.
    Subtree,
    /// Refuses the location itself and everything above it, but not
    /// what is under it. For the two locations the base makes private,
    /// the home and the temporary directory: a project must still be
    /// able to name `~/notes` or `/tmp/handoff` one path at a time,
    /// which a `Subtree` flavour would forbid. Also for the two files
    /// under the home, where "under" means nothing.
    Exact,
}

/// Every location `home`-relative or absolute that an extra path must
/// not name, reach into, or contain. Built per call because the
/// home-relative half depends on `home`; the list itself is short and
/// this runs once per `extra_paths` entry, not on a hot path.
fn guarded_locations(home: &str) -> [(String, Flavor, &'static str); 26] {
    let interop = "the interop interpreter and its sockets are \
                   `windows.interop`, which this version does not apply";
    let kernel = "kernel interfaces are not paths to grant";
    let system = "the system stays as the base mounts it";
    let willie_state = "Willie's own state stays as the base mounts it";
    let tools = "managed tools are `tools.ro`, read-only; an extra path \
                 must not reopen them read-write";
    let caches = "package caches are `caches.rw`, one per project; an \
                  extra path must not share them across projects";
    [
        ("/init".to_owned(), Flavor::Subtree, interop),
        ("/run".to_owned(), Flavor::Subtree, interop),
        ("/proc".to_owned(), Flavor::Subtree, kernel),
        ("/sys".to_owned(), Flavor::Subtree, kernel),
        ("/dev".to_owned(), Flavor::Subtree, kernel),
        ("/etc".to_owned(), Flavor::Subtree, system),
        ("/usr".to_owned(), Flavor::Subtree, system),
        ("/bin".to_owned(), Flavor::Subtree, system),
        ("/sbin".to_owned(), Flavor::Subtree, system),
        ("/lib".to_owned(), Flavor::Subtree, system),
        ("/lib64".to_owned(), Flavor::Subtree, system),
        ("/opt/willie".to_owned(), Flavor::Subtree, willie_state),
        ("/var/lib/willie".to_owned(), Flavor::Subtree, willie_state),
        // The two the base makes private, before what sits under them:
        // an entry for a path *under* the home matches the home first
        // through its ancestor half, and would answer with the wrong
        // reason.
        (
            "/tmp".to_owned(),
            Flavor::Exact,
            "the temporary directory is private to the session; grant \
             paths inside it one by one",
        ),
        (
            home.to_owned(),
            Flavor::Exact,
            "the home is private; grant paths inside it one by one",
        ),
        // After the home, so the home keeps its own reason: this entry
        // matches the home too, through its ancestor half.
        (
            format!("{home}/projects"),
            Flavor::Exact,
            "every project lives under it; grant the one you mean, not \
             all of them at once",
        ),
        (
            format!("{home}/.willie"),
            Flavor::Subtree,
            "Willie's per-user state holds the login and every \
             project's caches",
        ),
        (
            format!("{home}/.ssh"),
            Flavor::Subtree,
            "keys are `ssh`, which this version does not apply",
        ),
        (
            format!("{home}/.claude"),
            Flavor::Subtree,
            "the login is `agent.state`",
        ),
        (format!("{home}/.local"), Flavor::Subtree, tools),
        (format!("{home}/.dotnet"), Flavor::Subtree, tools),
        (format!("{home}/.npm"), Flavor::Subtree, caches),
        (format!("{home}/.nuget"), Flavor::Subtree, caches),
        (format!("{home}/.cache"), Flavor::Subtree, caches),
        (
            format!("{home}/.claude.json"),
            Flavor::Exact,
            "the login is `agent.state`",
        ),
        (
            format!("{home}/.gitconfig"),
            Flavor::Exact,
            "the identity is `git.identity`",
        ),
    ]
}

/// `/mnt` is not a system directory the base guards outright: it is
/// where Windows drives land, and the project's own is always bound.
/// Refuses `/mnt` itself (every drive is `mnt.all`, not applied), a
/// bare drive letter under it (the whole of that one drive, the same
/// capability), and anything under it whose first component is not a
/// single ASCII letter — the WSLg runtime directory chief among them,
/// a channel to the Windows side in the same family `/run` closes, not
/// a Windows drive at all. `path` must already be normalised.
fn guard_mnt(path: &str) -> Option<&'static str> {
    let rest = path.strip_prefix("/mnt")?;
    if rest.is_empty() {
        return Some(
            "every Windows drive is `mnt.all`, which this version does \
             not apply",
        );
    }
    let mut components = rest.strip_prefix('/')?.split('/');
    let drive = components.next().unwrap_or("");
    let is_drive_letter = drive.len() == 1
        && drive
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic());
    if !is_drive_letter {
        return Some(
            "this reaches the Windows side; interop is `windows.interop`, \
             which this version does not apply",
        );
    }
    if components.next().is_none() {
        return Some(
            "a whole Windows drive is `mnt.all`, which this version \
             does not apply",
        );
    }
    None
}

/// Why an extra path is refused, if it is. An extra path exists to
/// reach outside the project; it must not name, reach into, or contain
/// what the base closes (the system, the kernel's interfaces, the
/// interop interpreter, Willie's own state, the private home and
/// temporary directory, the managed tool roots and package caches) or
/// what a deferred capability grants on its own terms (a whole Windows
/// drive, the keys, the login). Public so the
/// app and `sandbox explain` can ask the same question `resolve`
/// answers, and so `willie-harness` can assert its own managed paths
/// are on this list.
///
/// The comparison is **textual and lexical**, after normalising both
/// `path` and `home`: repeated separators collapsed, `.` components
/// dropped, a trailing separator dropped. A `..` component is refused
/// outright rather than resolved lexically — see
/// [`has_dotdot_component`]. Guarding is symmetric: a location is
/// refused, everything under it, and everything above it, because an
/// ancestor of a guarded location contains it (so `/home`, `/var` and
/// `/opt` are refused along with what they contain, even though none
/// is itself on the list). The home and the temporary directory are
/// the exceptions, guarded exactly rather than as subtrees, or nothing
/// inside them — including the paths a project is meant to be able to
/// grant — could ever pass.
///
/// This guard does no I/O and cannot see whether an allowed path is
/// itself a symbolic link into a guarded one. That gap is real and is
/// not closed here: the supervisor owes a resolution-time re-check
/// immediately before it applies the plan, where I/O is allowed.
///
/// `/run` is not refused for tidiness: decision 0016 measured that the
/// interop interpreter stays registered with the kernel whether or not
/// `/init` is in the namespace, so an absent `/init` stops nothing by
/// itself. What actually keeps a Windows executable from running is
/// that `/run` — where the interop socket the interpreter dials lives
/// — is not in the namespace. Binding `/run` back in was measured to
/// make a Windows executable run and exit successfully, so an
/// `extra.paths` entry naming `/run` would not merely widen reach, it
/// would void this boundary's whole defence against Windows interop.
#[must_use]
pub fn guard_extra_path(path: &str, home: &str) -> Option<&'static str> {
    if has_dotdot_component(path) {
        return Some(
            "a `..` component is refused outright and never resolved; \
             name the path directly",
        );
    }
    let path = normalize_lexical(path);
    let home = normalize_lexical(home);

    if path == "/" {
        return Some("the whole filesystem");
    }
    if let Some(reason) = guard_mnt(&path) {
        return Some(reason);
    }
    for (location, flavor, reason) in guarded_locations(&home) {
        let matches = match flavor {
            Flavor::Subtree => {
                is_prefix_path(&location, &path)
                    || is_prefix_path(&path, &location)
            }
            Flavor::Exact => is_prefix_path(&path, &location),
        };
        if matches {
            return Some(reason);
        }
    }
    None
}

/// Merges layer 1 (the harness defaults) with layer 2 (the project
/// profile). Authority increases, so the profile wins where it speaks,
/// with one exception: `project.rw` is the session itself and cannot be
/// taken away. `extra_paths` does not merge at all: it comes from the
/// profile alone and is never inherited from layer 1, which has none.
///
/// Every refusal happens here, in the daemon, before any process
/// exists: a policy that cannot be applied must not become a session
/// that pretends it was. `home` is the real home; the guard on
/// `extra_paths` needs it to name the login, the keys and Willie's
/// state.
pub fn resolve(
    defaults: CapabilitySet,
    profile: &SandboxProfile,
    home: &str,
) -> Result<CapabilitySet, CapabilityError> {
    for (asked, capability) in [
        (profile.home_persistent, Capability::HomePersistent),
        (profile.ssh, Capability::Ssh),
        (profile.mnt_all, Capability::MntAll),
        (profile.windows_interop, Capability::WindowsInterop),
    ] {
        if asked == Some(true) {
            return Err(CapabilityError::Unsupported(capability));
        }
    }

    for extra in &profile.extra_paths {
        if !extra.path.starts_with('/') {
            return Err(CapabilityError::ExtraPathNotAbsolute {
                path: extra.path.clone(),
            });
        }
        if let Some(reason) = guard_extra_path(&extra.path, home) {
            return Err(CapabilityError::ExtraPathGuarded {
                path: extra.path.clone(),
                reason,
            });
        }
    }

    Ok(CapabilitySet {
        project_rw: true,
        agent_state: profile.agent_state.unwrap_or(defaults.agent_state),
        tools_ro: profile.tools_ro.unwrap_or(defaults.tools_ro),
        caches_rw: profile.caches_rw.unwrap_or(defaults.caches_rw),
        git_identity: profile.git_identity.unwrap_or(defaults.git_identity),
        extra_paths: profile.extra_paths.clone(),
    })
}

/// Where a capability's value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The harness decided it.
    Default,
    /// The project's profile spoke.
    Profile,
    /// This version cannot apply it at all.
    Unavailable,
}

/// One row of `sandbox explain`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Explained {
    pub capability: Capability,
    pub enabled: bool,
    pub source: Source,
}

/// What a session for this project would run under, capability by
/// capability, in the order the UI lists them. Resolves first, so a
/// profile that cannot be applied is refused here exactly as it is at
/// `session.create`. `home` is the real home, forwarded to `resolve`
/// unchanged: the guard on `extra_paths` needs it to name the login,
/// the keys and Willie's state.
pub fn explain(
    defaults: CapabilitySet,
    profile: &SandboxProfile,
    home: &str,
) -> Result<Vec<Explained>, CapabilityError> {
    let resolved = resolve(defaults, profile, home)?;

    Ok(Capability::ALL
        .iter()
        .map(|&capability| {
            let enabled = resolved.enabled(capability);
            let spoken = match capability {
                Capability::ProjectRw => false,
                Capability::AgentState => profile.agent_state.is_some(),
                Capability::ToolsRo => profile.tools_ro.is_some(),
                Capability::CachesRw => profile.caches_rw.is_some(),
                Capability::GitIdentity => profile.git_identity.is_some(),
                Capability::ExtraPaths => !profile.extra_paths.is_empty(),
                Capability::HomePersistent
                | Capability::Ssh
                | Capability::MntAll
                | Capability::WindowsInterop => false,
            };
            let source = if capability.is_implemented() {
                if spoken {
                    Source::Profile
                } else {
                    Source::Default
                }
            } else {
                Source::Unavailable
            };

            Explained {
                capability,
                enabled,
                source,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/willie";

    fn defaults() -> CapabilitySet {
        CapabilitySet {
            project_rw: true,
            agent_state: true,
            tools_ro: true,
            caches_rw: true,
            git_identity: true,
            extra_paths: Vec::new(),
        }
    }

    fn row(rows: &[Explained], c: Capability) -> &Explained {
        rows.iter()
            .find(|r| r.capability == c)
            .unwrap_or_else(|| panic!("{} missing", c.display_name()))
    }

    #[test]
    fn explain_reports_a_default_and_an_override_apart() {
        let profile = SandboxProfile {
            agent_state: Some(false),
            ..SandboxProfile::default()
        };

        let rows = explain(defaults(), &profile, HOME).unwrap();

        let credential = row(&rows, Capability::AgentState);
        assert!(!credential.enabled);
        assert_eq!(credential.source, Source::Profile);

        let tools = row(&rows, Capability::ToolsRo);
        assert!(tools.enabled);
        assert_eq!(tools.source, Source::Default);
    }

    /// A capability this version cannot apply is not "off": off invites
    /// switching it on. It is unavailable, and it is listed so that what
    /// is coming is visible without being offered.
    #[test]
    fn explain_lists_a_deferred_capability_as_unavailable() {
        let rows =
            explain(defaults(), &SandboxProfile::default(), HOME).unwrap();

        let ssh = row(&rows, Capability::Ssh);
        assert!(!ssh.enabled);
        assert_eq!(ssh.source, Source::Unavailable);
    }

    #[test]
    fn explain_reports_every_capability_once_in_declaration_order() {
        let rows =
            explain(defaults(), &SandboxProfile::default(), HOME).unwrap();

        let listed: Vec<Capability> =
            rows.iter().map(|r| r.capability).collect();

        assert_eq!(listed, Capability::ALL);
    }

    /// The same refusal the create path gives: one condition, one code.
    #[test]
    fn explain_refuses_a_profile_that_cannot_resolve() {
        let profile = SandboxProfile {
            ssh: Some(true),
            ..SandboxProfile::default()
        };

        assert_eq!(
            explain(defaults(), &profile, HOME).unwrap_err(),
            CapabilityError::Unsupported(Capability::Ssh)
        );
    }

    #[test]
    fn an_empty_profile_leaves_the_defaults_alone() {
        let set =
            resolve(defaults(), &SandboxProfile::default(), HOME).unwrap();

        assert_eq!(set, defaults());
    }

    #[test]
    fn a_profile_turns_one_capability_off_without_touching_the_rest() {
        let profile = SandboxProfile {
            agent_state: Some(false),
            ..SandboxProfile::default()
        };

        let set = resolve(defaults(), &profile, HOME).unwrap();

        assert!(!set.agent_state);
        assert!(set.tools_ro);
        assert!(set.caches_rw);
        assert!(set.git_identity);
    }

    #[test]
    fn a_profile_turns_one_capability_on_that_the_harness_left_off() {
        let mut base = defaults();
        base.caches_rw = false;
        let profile = SandboxProfile {
            caches_rw: Some(true),
            ..SandboxProfile::default()
        };

        assert!(resolve(base, &profile, HOME).unwrap().caches_rw);
    }

    /// `project.rw` is the session itself: a profile cannot take it
    /// away, because a session without its project has no purpose.
    #[test]
    fn the_project_bind_survives_a_profile_that_disables_it() {
        let profile = SandboxProfile {
            project_rw: Some(false),
            ..SandboxProfile::default()
        };

        assert!(resolve(defaults(), &profile, HOME).unwrap().project_rw);
    }

    #[test]
    fn a_profile_naming_a_deferred_capability_is_refused_by_name() {
        for (profile, expected) in [
            (
                SandboxProfile {
                    ssh: Some(true),
                    ..SandboxProfile::default()
                },
                Capability::Ssh,
            ),
            (
                SandboxProfile {
                    windows_interop: Some(true),
                    ..SandboxProfile::default()
                },
                Capability::WindowsInterop,
            ),
            (
                SandboxProfile {
                    mnt_all: Some(true),
                    ..SandboxProfile::default()
                },
                Capability::MntAll,
            ),
            (
                SandboxProfile {
                    home_persistent: Some(true),
                    ..SandboxProfile::default()
                },
                Capability::HomePersistent,
            ),
        ] {
            let err = resolve(defaults(), &profile, HOME).unwrap_err();

            assert_eq!(err, CapabilityError::Unsupported(expected));
            assert_eq!(err.code(), "sandbox_capability_unsupported");
            assert!(err.to_string().contains(expected.display_name()));
        }
    }

    /// Disabling one costs nothing: it is already off, and refusing
    /// would make a profile that asks for less than Willie gives fail.
    #[test]
    fn a_profile_disabling_a_deferred_capability_is_accepted() {
        let profile = SandboxProfile {
            ssh: Some(false),
            mnt_all: Some(false),
            ..SandboxProfile::default()
        };

        assert!(resolve(defaults(), &profile, HOME).is_ok());
    }

    #[test]
    fn extra_paths_reach_the_resolved_set_in_the_order_given() {
        let profile = SandboxProfile {
            extra_paths: vec![
                ExtraPath {
                    path: "/srv/shared".into(),
                    mode: PathMode::Ro,
                },
                ExtraPath {
                    path: "/srv/out".into(),
                    mode: PathMode::Rw,
                },
            ],
            ..SandboxProfile::default()
        };

        let set = resolve(defaults(), &profile, HOME).unwrap();

        assert_eq!(set.extra_paths.len(), 2);
        assert_eq!(set.extra_paths[0].mode, PathMode::Ro);
        assert_eq!(set.extra_paths[1].path, "/srv/out");
    }

    /// A relative path would bind whatever the supervisor's working
    /// directory happens to be, so it is refused where it is cheap to
    /// refuse: before any process exists.
    #[test]
    fn a_relative_extra_path_is_refused() {
        let profile = SandboxProfile {
            extra_paths: vec![ExtraPath {
                path: "srv/shared".into(),
                mode: PathMode::Ro,
            }],
            ..SandboxProfile::default()
        };

        let err = resolve(defaults(), &profile, HOME).unwrap_err();

        assert_eq!(err.code(), "sandbox_profile_invalid");
        assert!(err.remediation().contains("absolute"));
    }

    /// A spec written before sandboxing deserialises to this. It is the
    /// tightest policy there is now that the boundary is applied — a
    /// private home, the workspace, the harness binary — and it must
    /// still never mean "reach nothing", which would be a session that
    /// cannot see its project.
    #[test]
    fn a_defaulted_set_still_carries_the_project_bind() {
        let set = CapabilitySet::default();

        assert!(set.project_rw);
        assert!(!set.agent_state);
        assert!(!set.tools_ro);
        assert!(set.extra_paths.is_empty());
    }

    #[test]
    fn every_capability_has_a_unique_name_and_a_consequence() {
        let mut names: Vec<&str> =
            Capability::ALL.iter().map(|c| c.display_name()).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();

        assert_eq!(names.len(), total, "duplicate name in {names:?}");
        for c in Capability::ALL {
            assert!(!c.consequence().is_empty(), "{}", c.display_name());
        }
    }

    #[test]
    fn exactly_the_six_shipped_capabilities_are_implemented() {
        let implemented: Vec<&str> = Capability::ALL
            .iter()
            .filter(|c| c.is_implemented())
            .map(|c| c.display_name())
            .collect();

        assert_eq!(
            implemented,
            [
                "project.rw",
                "agent.state",
                "tools.ro",
                "caches.rw",
                "git.identity",
                "extra.paths",
            ]
        );
    }

    /// The profile is a hand-editable file as well as something the UI
    /// writes, so its keys are part of the contract.
    #[test]
    fn a_profile_round_trips_through_toml_with_snake_case_keys() {
        let text = "\
agent_state = false
extra_paths = [{ path = \"/srv/shared\", mode = \"ro\" }]
";

        let profile: SandboxProfile = toml::from_str(text).unwrap();

        assert_eq!(profile.agent_state, Some(false));
        assert_eq!(profile.extra_paths.len(), 1);
        assert_eq!(profile.extra_paths[0].mode, PathMode::Ro);
    }

    #[test]
    fn an_unknown_profile_key_is_refused_rather_than_ignored() {
        let text = "agnt_state = false\n";

        let err = toml::from_str::<SandboxProfile>(text).unwrap_err();

        assert!(err.to_string().contains("agnt_state"), "{err}");
    }

    /// Every reader of a set — `explain`, the app's catalogue — asks
    /// this one question, so a capability with no field must answer
    /// `false` rather than let a caller guess.
    #[test]
    fn a_set_answers_for_every_capability_and_grants_no_deferred_one() {
        let set = CapabilitySet {
            agent_state: false,
            extra_paths: vec![ExtraPath {
                path: "/srv/shared".into(),
                mode: PathMode::Ro,
            }],
            ..defaults()
        };

        assert!(set.enabled(Capability::ProjectRw));
        assert!(!set.enabled(Capability::AgentState));
        assert!(set.enabled(Capability::ToolsRo));
        assert!(set.enabled(Capability::ExtraPaths));
        assert!(!CapabilitySet::default().enabled(Capability::ExtraPaths));
        for deferred in [
            Capability::HomePersistent,
            Capability::Ssh,
            Capability::MntAll,
            Capability::WindowsInterop,
        ] {
            assert!(!set.enabled(deferred), "{}", deferred.display_name());
        }
    }

    fn extra(path: &str) -> SandboxProfile {
        SandboxProfile {
            extra_paths: vec![ExtraPath {
                path: path.into(),
                mode: PathMode::Ro,
            }],
            ..SandboxProfile::default()
        }
    }

    /// An extra path reaches outside the project on purpose; what it
    /// must not do is hand back what the base closes or a deferred
    /// capability will grant on its own terms.
    #[test]
    fn an_extra_path_that_voids_the_boundary_is_refused_with_its_reason() {
        for path in [
            "/",
            "/mnt",
            "/mnt/c",
            "/init",
            "/run",
            "/run/WSL",
            "/proc",
            "/sys/kernel",
            "/dev",
            "/etc",
            "/etc/shadow",
            "/usr",
            "/usr/bin",
            "/bin",
            "/sbin",
            "/lib",
            "/lib64",
            "/opt/willie",
            "/opt/willie/bin",
            "/var/lib/willie",
            "/var/lib/willie/projects",
            "/home/willie",
            "/home/willie/.willie",
            "/home/willie/.willie/agent-state/claude",
            "/home/willie/.ssh",
            "/home/willie/.ssh/id_ed25519",
            "/home/willie/.claude",
            "/home/willie/.claude.json",
            "/home/willie/.gitconfig",
            // A path is compared normalised, not as written: a double
            // separator, a `.` component or a trailing separator must
            // not walk it past an arm the plain spelling would hit.
            "//mnt",
            "//run",
            "/mnt//c",
            "/mnt/./c",
            "//home/willie/.ssh",
            // `..` is refused outright rather than resolved: this
            // would lexically reach `.ssh`, but is never given the
            // chance to.
            "/home/willie/notes/../.ssh",
            // An ancestor of a guarded location contains it, so
            // granting the ancestor must be refused too, even though
            // none of these is itself on the guarded list.
            "/home",
            "/var",
            "/var/lib",
            "/opt",
            // `.claude` guards its contents now, not only its own
            // name: the credential lives inside it.
            "/home/willie/.claude/projects",
            // The managed tool roots and package caches: an extra
            // path must not reopen read-write what `tools.ro` binds
            // read-only or hand every project the same `caches.rw`
            // copy.
            "/home/willie/.local",
            "/home/willie/.dotnet",
            "/home/willie/.npm",
            "/home/willie/.nuget",
            "/home/willie/.cache",
            // The private temporary directory the base gives every
            // session: an extra path naming it would bind the shared
            // one over it, because the base renders first and a later
            // bind at the same destination wins. Every spelling of it,
            // since the comparison is normalised.
            "/tmp",
            "/tmp/",
            "//tmp",
            "/tmp/.",
            // Not a Windows drive: a channel to the Windows side in
            // the same family `/run` closes.
            "/mnt/wsl",
            "/mnt/wslg/runtime-dir",
            // Every workspace lives directly under this one, and an
            // extra path renders after the workspace's own bind, so
            // naming it would mount the whole tree over the session's
            // project and hand that session every other project at
            // once. The last place a single entry could still override
            // what the base mounted.
            "/home/willie/projects",
            "/home/willie/projects/",
            "//home/willie/projects",
        ] {
            let err = resolve(defaults(), &extra(path), HOME).unwrap_err();

            assert_eq!(err.code(), "sandbox_profile_invalid", "{path}");
            assert!(
                matches!(&err, CapabilityError::ExtraPathGuarded { path: p, .. } if p == path),
                "{path}: {err:?}"
            );
            assert!(err.to_string().contains(path), "{err}");
        }
    }

    #[test]
    fn an_extra_path_that_merely_reaches_outside_the_project_is_allowed() {
        for path in [
            "/srv/shared",
            "/mnt/c/Users/me/data",
            "/mnt/d/out",
            "/home/willie/projects/other",
            "/home/willie/notes",
            // Inside the private temporary directory, which is guarded
            // exactly so a hand-off point stays grantable.
            "/tmp/handoff",
            "/var/lib/other-tool",
            "/opt/tools",
            // Siblings that merely share a prefix with a guarded name
            // must not be caught by a boundary that compares
            // characters instead of path components.
            "/home/willie/.claude-old",
            "/optical",
            "/variable",
            "/homework",
        ] {
            assert!(resolve(defaults(), &extra(path), HOME).is_ok(), "{path}");
        }
    }

    /// The reason is not decoration: it is the sentence the dialog and
    /// the session's failure show. Each flavour of guard must answer
    /// with its own, and an entry must not be shadowed into answering
    /// with a neighbour's.
    #[test]
    fn each_flavour_of_guard_answers_with_the_reason_a_user_reads() {
        for (path, expected) in [
            // A subtree: the location, what is under it, what is above.
            ("/var/lib/willie", "Willie's own state"),
            ("/var/lib/willie/projects", "Willie's own state"),
            ("/var", "Willie's own state"),
            ("/home/willie/.npm", "one per project"),
            // Guarded exactly: the location and its ancestors, while
            // what is under it stays grantable.
            ("/home/willie", "the home is private"),
            ("/home", "the home is private"),
            ("/tmp", "the temporary directory is private"),
            ("//tmp", "the temporary directory is private"),
            ("/home/willie/projects", "every project lives under it"),
            // Still the home's own answer, not the projects root's:
            // the home is guarded first precisely so that the nearest
            // reason is the one a user reads.
            ("/home/willie/", "the home is private"),
            // The Windows mount root, guarded by its own rules.
            ("/mnt", "every Windows drive is `mnt.all`"),
            ("/mnt/c", "a whole Windows drive"),
            ("/mnt/wsl", "this reaches the Windows side"),
            // The two answered before the list is walked at all.
            ("/", "the whole filesystem"),
            ("/home/willie/notes/../.ssh", "`..` component"),
        ] {
            let err = resolve(defaults(), &extra(path), HOME).unwrap_err();

            let CapabilityError::ExtraPathGuarded { reason, .. } = &err else {
                panic!("{path}: {err:?}");
            };
            assert!(reason.contains(expected), "{path}: {reason}");
            assert!(err.remediation().contains(expected), "{path}");
        }
    }

    /// The same guard through `explain`, so the dialog refuses what the
    /// session would refuse.
    #[test]
    fn explain_refuses_a_guarded_extra_path_too() {
        assert!(matches!(
            explain(defaults(), &extra("/mnt"), HOME).unwrap_err(),
            CapabilityError::ExtraPathGuarded { .. }
        ));
    }
}
