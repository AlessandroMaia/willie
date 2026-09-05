//! The boundary a session runs inside, as data.
//!
//! `plan` turns the policy a spec carries into the mounts the session
//! gets; `bwrap::argv` turns those into the helper's argument vector.
//! Nothing here forks, mounts or reads the disk, so all of it is tested
//! on any host: the supervisor applies a plan, and the daemon can show
//! one without starting a session.

pub mod bwrap;
pub mod inner;

use std::{collections::BTreeMap, fmt, path::Path};

use willie_core::{sandbox::PathMode, session::SessionSpec};
use willie_harness::{Harness, cache_paths, tool_roots};

/// One change to the session's view of the filesystem. Applied in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// A fresh, empty filesystem at `dest`, with these permissions.
    Tmpfs { dest: String, perms: &'static str },
    /// The host path `src` visible at `dest`. `optional` tolerates a
    /// missing `src`: the plan names a layout, not what this machine has
    /// installed so far.
    Bind {
        src: String,
        dest: String,
        mode: PathMode,
        optional: bool,
    },
    /// A symbolic link at `link` that says `target`.
    Symlink { target: String, link: String },
}

/// What one session gets, resolved from its spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The session's home: a private, empty filesystem at the real
    /// home's path, so every path the harness expects still resolves.
    pub home: String,
    /// The working directory, bound read-write at its own path.
    pub workspace: String,
    /// The harness command, untouched.
    pub argv: Vec<String>,
    /// The session's whole environment, from the spec. The helper clears
    /// what it inherits and sets exactly this, so the boundary does not
    /// depend on how the supervisor was started.
    pub env: BTreeMap<String, String>,
    /// Host directories that must exist before the ops are applied:
    /// the per-project caches. A session never starts without them.
    pub ensure_dirs: Vec<String>,
    pub ops: Vec<Op>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// The spec's environment names no `HOME`, so there is nothing to
    /// make private.
    HomeMissing,
}

impl PlanError {
    /// The spec is the daemon's to write; a spec the supervisor cannot
    /// plan from is the same class of failure as one it cannot parse.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::HomeMissing => "spec_invalid",
        }
    }
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeMissing => {
                write!(f, "spec.env names no HOME to make private")
            }
        }
    }
}

impl std::error::Error for PlanError {}

fn bind(src: &str, dest: &str, mode: PathMode, optional: bool) -> Op {
    Op::Bind {
        src: src.to_owned(),
        dest: dest.to_owned(),
        mode,
        optional,
    }
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Whether `path` is `prefix` or lies under it. Deliberately a plain
/// comparison: both sides come from the plan itself, which the daemon
/// resolved and wrote into the spec, so there is nothing here to
/// normalise. Not the policy's guard, which answers a different
/// question about a string a user typed.
fn under(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The system tree the base binds read-only whatever the policy says,
/// so anything installed inside it is already reachable.
const SYSTEM_TREE: &str = "/usr";

/// The mounts a session gets under its policy. The private home comes
/// first, because everything the policy grants inside it is mounted on
/// top; the project follows, because a session is its project; the
/// harness binary is always reachable, because a session that cannot
/// start is not a tighter policy, it is no session at all.
pub fn plan(
    spec: &SessionSpec,
    harness: &dyn Harness,
) -> Result<Plan, PlanError> {
    let home = spec
        .env
        .get("HOME")
        .filter(|h| !h.is_empty())
        .cloned()
        .ok_or(PlanError::HomeMissing)?;
    let home_path = Path::new(&home);
    let policy = &spec.capabilities;
    let mut ensure_dirs = Vec::new();
    let mut ops = vec![Op::Tmpfs {
        dest: home.clone(),
        perms: "0700",
    }];

    ops.push(bind(&spec.workspace, &spec.workspace, PathMode::Rw, false));

    if policy.agent_state
        && let Some(state) = harness.agent_state(home_path)
    {
        let dir = text(&state.dir);
        ops.push(bind(&dir, &dir, PathMode::Rw, false));
        for link in state.links {
            ops.push(Op::Symlink {
                target: link.target,
                link: text(&link.path),
            });
        }
    }

    let mut carried = vec![SYSTEM_TREE.to_owned()];
    if policy.tools_ro {
        for root in tool_roots(home_path) {
            let root = text(&root);
            ops.push(bind(&root, &root, PathMode::Ro, true));
            carried.push(root);
        }
    }

    // The harness is what the session runs, so a policy that hides the
    // tools must still let it start. Where something already carries it
    // the bind is not merely redundant, it refuses the session: the
    // official installer makes the binary a symbolic link into a
    // versioned directory, and the helper will not mount over a symlink
    // that a read-only bind has already put there.
    if let Some(binary) = spec.argv.first()
        && !carried.iter().any(|root| under(binary, root))
    {
        ops.push(bind(binary, binary, PathMode::Ro, false));
    }

    if policy.caches_rw {
        for cache in cache_paths(home_path) {
            let src = format!(
                "{home}/.willie/caches/{}/{}",
                spec.project_id, cache.name
            );
            ops.push(bind(&src, &text(&cache.path), PathMode::Rw, false));
            ensure_dirs.push(src);
        }
    }

    if policy.git_identity {
        let config = format!("{home}/.gitconfig");
        ops.push(bind(&config, &config, PathMode::Ro, false));
    }

    for extra in &policy.extra_paths {
        ops.push(bind(&extra.path, &extra.path, extra.mode, false));
    }

    Ok(Plan {
        home,
        workspace: spec.workspace.clone(),
        argv: spec.argv.clone(),
        env: spec.env.clone(),
        ensure_dirs,
        ops,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use willie_core::{
        id::{ProjectId, SessionId},
        sandbox::{CapabilitySet, ExtraPath, PathMode},
        session::SessionSpec,
    };
    use willie_harness::{ClaudeCode, Harness};

    use super::*;

    const HOME: &str = "/home/willie";
    const WS: &str = "/home/willie/projects/x";
    const BIN: &str = "/home/willie/.local/bin/claude";

    fn spec_with(capabilities: CapabilitySet) -> SessionSpec {
        let mut env = BTreeMap::new();
        env.insert("HOME".to_owned(), HOME.to_owned());
        env.insert(
            "PATH".to_owned(),
            "/home/willie/.local/bin:/usr/bin".into(),
        );
        SessionSpec {
            id: SessionId::new(),
            project_id: ProjectId::new(),
            harness: "claude-code".into(),
            workspace: WS.into(),
            socket: "/run/willie/sessions/s.sock".into(),
            argv: vec![BIN.into(), "--continue".into()],
            env,
            created_at: "1".into(),
            willie_version: "0".into(),
            resumed_from: None,
            capabilities,
        }
    }

    fn all_on() -> CapabilitySet {
        ClaudeCode.default_capabilities()
    }

    fn planned(capabilities: CapabilitySet) -> (SessionSpec, Plan) {
        let spec = spec_with(capabilities);
        let plan = plan(&spec, &ClaudeCode).expect("a plan");
        (spec, plan)
    }

    fn bind(src: &str, dest: &str, mode: PathMode, optional: bool) -> Op {
        Op::Bind {
            src: src.into(),
            dest: dest.into(),
            mode,
            optional,
        }
    }

    /// Every path an op names, so a test can say "nowhere".
    fn mentioned(plan: &Plan) -> Vec<&str> {
        plan.ops
            .iter()
            .flat_map(|op| match op {
                Op::Tmpfs { dest, .. } => vec![dest.as_str()],
                Op::Bind { src, dest, .. } => vec![src.as_str(), dest.as_str()],
                Op::Symlink { target, link } => {
                    vec![target.as_str(), link.as_str()]
                }
            })
            .collect()
    }

    /// The real home is never visible: the session gets a fresh one, and
    /// it has to exist before anything is mounted inside it.
    #[test]
    fn the_home_is_a_private_tmpfs_mounted_before_anything_inside_it() {
        let (_, plan) = planned(all_on());

        assert_eq!(
            plan.ops[0],
            Op::Tmpfs {
                dest: HOME.into(),
                perms: "0700",
            }
        );
        assert_eq!(plan.home, HOME);
        assert!(plan.ops[1..].iter().any(|op| matches!(
            op, Op::Bind { dest, .. } if dest.starts_with("/home/willie/")
        )));
    }

    #[test]
    fn the_workspace_is_read_write_at_its_own_path_and_is_the_working_directory()
     {
        let (_, plan) = planned(all_on());

        assert!(plan.ops.contains(&bind(WS, WS, PathMode::Rw, false)));
        assert_eq!(plan.workspace, WS);
    }

    #[test]
    fn the_plan_carries_the_harness_argv_verbatim() {
        let (spec, plan) = planned(all_on());

        assert_eq!(plan.argv, spec.argv);
    }

    /// The daemon resolved the environment when it wrote the spec; the
    /// plan carries it, it does not rebuild it.
    #[test]
    fn the_plan_carries_the_spec_environment_verbatim() {
        let (spec, plan) = planned(all_on());

        assert_eq!(plan.env, spec.env);
    }

    /// Binding the harness again over a directory that already carries
    /// it is not merely redundant, it refuses the session: the official
    /// installer makes the binary a symbolic link into a versioned
    /// directory, and the helper will not mount over a symlink. Measured
    /// against a real installation, where it read
    /// `Can't mount on symlink destination`.
    #[test]
    fn the_harness_binary_is_left_to_the_tool_root_that_already_carries_it() {
        let (_, plan) = planned(all_on());

        assert!(plan.ops.contains(&bind(
            "/home/willie/.local",
            "/home/willie/.local",
            PathMode::Ro,
            true
        )));
        assert!(
            !plan
                .ops
                .iter()
                .any(|op| matches!(op, Op::Bind { dest, .. } if dest == BIN)),
            "{:?}",
            plan.ops
        );
    }

    /// The base binds the system tree read-only whatever the policy
    /// says, so a harness installed there is already reachable and the
    /// same refusal would apply to a symlink under it.
    #[test]
    fn a_harness_under_the_system_tree_is_left_to_the_base() {
        let mut spec = spec_with(CapabilitySet {
            tools_ro: false,
            ..all_on()
        });
        spec.argv = vec!["/usr/local/bin/claude".into()];

        let plan = plan(&spec, &ClaudeCode).expect("a plan");

        assert!(
            !plan.ops.iter().any(|op| matches!(
                op, Op::Bind { dest, .. } if dest == "/usr/local/bin/claude"
            )),
            "{:?}",
            plan.ops
        );
    }

    /// The harness is what the session runs; a policy that hides the
    /// tools must still let it start, or `tools.ro` off means no session.
    #[test]
    fn the_harness_binary_is_reachable_even_when_the_tools_are_not() {
        let (_, plan) = planned(CapabilitySet {
            tools_ro: false,
            ..all_on()
        });

        assert!(plan.ops.contains(&bind(BIN, BIN, PathMode::Ro, false)));
        assert!(!mentioned(&plan).contains(&"/home/willie/.local"));
    }

    #[test]
    fn agent_state_binds_the_login_directory_and_recreates_its_links() {
        let (_, plan) = planned(all_on());
        let state = "/home/willie/.willie/agent-state/claude";

        let dir = plan
            .ops
            .iter()
            .position(|op| *op == bind(state, state, PathMode::Rw, false))
            .expect("the login directory is bound read-write");
        assert_eq!(
            plan.ops[dir + 1],
            Op::Symlink {
                target: ".willie/agent-state/claude/dot-claude".into(),
                link: "/home/willie/.claude".into(),
            }
        );
        assert_eq!(
            plan.ops[dir + 2],
            Op::Symlink {
                target: ".willie/agent-state/claude/claude.json".into(),
                link: "/home/willie/.claude.json".into(),
            }
        );
    }

    #[test]
    fn without_agent_state_no_login_path_appears_anywhere() {
        let (_, plan) = planned(CapabilitySet {
            agent_state: false,
            ..all_on()
        });

        for path in mentioned(&plan) {
            assert!(!path.contains(".claude"), "{path}");
            assert!(!path.contains("agent-state"), "{path}");
        }
    }

    /// A tool root that is not installed yet is not an error: the
    /// layout is fixed, the machine is not.
    #[test]
    fn tools_ro_binds_each_tool_root_read_only_and_tolerates_a_missing_one() {
        let (_, plan) = planned(all_on());

        for root in ["/home/willie/.local", "/home/willie/.dotnet"] {
            assert!(
                plan.ops.contains(&bind(root, root, PathMode::Ro, true)),
                "{root}"
            );
        }
    }

    /// A shared writable cache is where one project's compromise reaches
    /// the next, so each project gets its own copy over every cache path,
    /// and the supervisor is told to create it: a session never starts
    /// without its caches.
    #[test]
    fn caches_rw_binds_a_per_project_directory_over_each_cache_and_asks_for_it_to_exist()
     {
        let (spec, plan) = planned(all_on());
        let root = format!("/home/willie/.willie/caches/{}", spec.project_id);

        let expected: Vec<(String, &str)> = vec![
            (format!("{root}/npm"), "/home/willie/.npm"),
            (format!("{root}/nuget"), "/home/willie/.nuget"),
            (format!("{root}/cache"), "/home/willie/.cache"),
        ];
        for (src, dest) in &expected {
            assert!(
                plan.ops.contains(&bind(src, dest, PathMode::Rw, false)),
                "{src} over {dest}"
            );
        }
        assert_eq!(
            plan.ensure_dirs,
            expected
                .iter()
                .map(|(src, _)| src.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn without_caches_rw_nothing_is_created_and_no_cache_path_is_bound() {
        let (_, plan) = planned(CapabilitySet {
            caches_rw: false,
            ..all_on()
        });

        assert!(plan.ensure_dirs.is_empty());
        for path in mentioned(&plan) {
            assert!(!path.contains("caches"), "{path}");
            assert!(!path.ends_with("/.npm"), "{path}");
        }
    }

    #[test]
    fn git_identity_binds_the_configuration_read_only() {
        let cfg = "/home/willie/.gitconfig";
        let (_, with) = planned(all_on());
        let (_, without) = planned(CapabilitySet {
            git_identity: false,
            ..all_on()
        });

        assert!(with.ops.contains(&bind(cfg, cfg, PathMode::Ro, false)));
        assert!(!mentioned(&without).contains(&cfg));
    }

    #[test]
    fn extra_paths_are_bound_at_their_own_path_in_the_mode_and_order_given() {
        let (_, plan) = planned(CapabilitySet {
            extra_paths: vec![
                ExtraPath {
                    path: "/srv/shared".into(),
                    mode: PathMode::Ro,
                },
                ExtraPath {
                    path: "/mnt/c/out".into(),
                    mode: PathMode::Rw,
                },
            ],
            ..all_on()
        });

        let shared = plan
            .ops
            .iter()
            .position(|op| {
                *op == bind("/srv/shared", "/srv/shared", PathMode::Ro, false)
            })
            .expect("the read-only extra path");
        assert_eq!(
            plan.ops[shared + 1],
            bind("/mnt/c/out", "/mnt/c/out", PathMode::Rw, false)
        );
    }

    /// A spec written before sandboxing resolves to the defaulted set:
    /// the project and the harness, and nothing else of the machine.
    #[test]
    fn the_default_policy_reaches_only_the_project_and_its_harness() {
        let (_, plan) = planned(CapabilitySet::default());

        assert_eq!(
            plan.ops,
            vec![
                Op::Tmpfs {
                    dest: HOME.into(),
                    perms: "0700",
                },
                bind(WS, WS, PathMode::Rw, false),
                bind(BIN, BIN, PathMode::Ro, false),
            ]
        );
        assert!(plan.ensure_dirs.is_empty());
    }

    /// The boundary's whole point: what the policy does not name, the
    /// session does not see: the Windows drives, the interop
    /// interpreter, and Willie's own state and sockets.
    #[test]
    fn no_op_reaches_the_windows_drives_or_willie_state_unless_named() {
        let (_, plan) = planned(all_on());

        for path in mentioned(&plan) {
            for forbidden in ["/mnt", "/init", "/run", "/var/lib/willie"] {
                assert!(!path.starts_with(forbidden), "{path}");
            }
        }
    }

    /// Without a home there is nothing to make private; refusing is the
    /// closed direction.
    #[test]
    fn a_spec_without_a_home_is_refused() {
        let mut spec = spec_with(all_on());
        spec.env.remove("HOME");

        assert_eq!(
            plan(&spec, &ClaudeCode).unwrap_err(),
            PlanError::HomeMissing
        );
    }

    /// A harness with no login layout grants nothing under `agent.state`,
    /// rather than binding a guessed directory.
    #[test]
    fn a_harness_without_a_login_layout_binds_none_even_when_asked() {
        #[derive(Debug)]
        struct Quiet;

        impl Harness for Quiet {
            fn id(&self) -> &'static str {
                "quiet"
            }
            fn binary_name(&self) -> &'static str {
                "quiet"
            }
            fn capabilities(&self) -> willie_harness::HarnessCapabilities {
                willie_harness::HarnessCapabilities {
                    interactive_tui: false,
                    resume: willie_harness::Resume::None,
                    headless_stream: false,
                }
            }
            fn installer(&self) -> &'static str {
                "true"
            }
        }

        let spec = spec_with(CapabilitySet {
            agent_state: true,
            ..CapabilitySet::default()
        });

        let plan = plan(&spec, &Quiet).expect("a plan");

        assert!(!plan.ops.iter().any(|op| matches!(op, Op::Symlink { .. })));
        assert_eq!(plan.ops.len(), 3);
    }
}
