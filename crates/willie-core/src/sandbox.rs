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

/// A defaulted set means "written before sandboxing", which is not the
/// same as "reach nothing": the project bind is what makes a session a
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
}

impl CapabilityError {
    /// Stable machine-readable code (see docs/PROTOCOL.md).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unsupported(_) => "sandbox_capability_unsupported",
            Self::ExtraPathNotAbsolute { .. } => "sandbox_profile_invalid",
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
        }
    }
}

/// Merges layer 1 (the harness defaults) with layer 2 (the project
/// profile). Authority increases, so the profile wins where it speaks,
/// with one exception: `project.rw` is the session itself and cannot be
/// taken away. `extra_paths` does not merge at all: it comes from the
/// profile alone and is never inherited from layer 1, which has none.
///
/// Every refusal happens here, in the daemon, before any process
/// exists: a policy that cannot be applied must not become a session
/// that pretends it was.
pub fn resolve(
    defaults: CapabilitySet,
    profile: &SandboxProfile,
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
/// `session.create`.
pub fn explain(
    defaults: CapabilitySet,
    profile: &SandboxProfile,
) -> Result<Vec<Explained>, CapabilityError> {
    let resolved = resolve(defaults, profile)?;

    Ok(Capability::ALL
        .iter()
        .map(|&capability| {
            let (enabled, spoken) = match capability {
                Capability::ProjectRw => (resolved.project_rw, false),
                Capability::AgentState => {
                    (resolved.agent_state, profile.agent_state.is_some())
                }
                Capability::ToolsRo => {
                    (resolved.tools_ro, profile.tools_ro.is_some())
                }
                Capability::CachesRw => {
                    (resolved.caches_rw, profile.caches_rw.is_some())
                }
                Capability::GitIdentity => {
                    (resolved.git_identity, profile.git_identity.is_some())
                }
                Capability::ExtraPaths => (
                    !resolved.extra_paths.is_empty(),
                    !profile.extra_paths.is_empty(),
                ),
                Capability::HomePersistent
                | Capability::Ssh
                | Capability::MntAll
                | Capability::WindowsInterop => (false, false),
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

        let rows = explain(defaults(), &profile).unwrap();

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
        let rows = explain(defaults(), &SandboxProfile::default()).unwrap();

        let ssh = row(&rows, Capability::Ssh);
        assert!(!ssh.enabled);
        assert_eq!(ssh.source, Source::Unavailable);
    }

    #[test]
    fn explain_reports_every_capability_once_in_declaration_order() {
        let rows = explain(defaults(), &SandboxProfile::default()).unwrap();

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
            explain(defaults(), &profile).unwrap_err(),
            CapabilityError::Unsupported(Capability::Ssh)
        );
    }

    #[test]
    fn an_empty_profile_leaves_the_defaults_alone() {
        let set = resolve(defaults(), &SandboxProfile::default()).unwrap();

        assert_eq!(set, defaults());
    }

    #[test]
    fn a_profile_turns_one_capability_off_without_touching_the_rest() {
        let profile = SandboxProfile {
            agent_state: Some(false),
            ..SandboxProfile::default()
        };

        let set = resolve(defaults(), &profile).unwrap();

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

        assert!(resolve(base, &profile).unwrap().caches_rw);
    }

    /// `project.rw` is the session itself: a profile cannot take it
    /// away, because a session without its project has no purpose.
    #[test]
    fn the_project_bind_survives_a_profile_that_disables_it() {
        let profile = SandboxProfile {
            project_rw: Some(false),
            ..SandboxProfile::default()
        };

        assert!(resolve(defaults(), &profile).unwrap().project_rw);
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
            let err = resolve(defaults(), &profile).unwrap_err();

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

        assert!(resolve(defaults(), &profile).is_ok());
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

        let set = resolve(defaults(), &profile).unwrap();

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

        let err = resolve(defaults(), &profile).unwrap_err();

        assert_eq!(err.code(), "sandbox_profile_invalid");
        assert!(err.remediation().contains("absolute"));
    }

    /// A spec written before sandboxing deserialises to this, so it
    /// must mean "unconfined, as it used to be" and never "reach
    /// nothing", which would be a session that cannot see its project.
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
}
