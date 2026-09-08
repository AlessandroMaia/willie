//! The engine facade the desktop app talks to. Owns the distro manager
//! and the daemon supervisor; every method returns the new status or a
//! typed error so the UI never has to interpret anything.

use std::{path::PathBuf, sync::mpsc::Receiver};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use willie_core::{
    id::{JobId, ProjectId, SessionId},
    project::Project,
    sandbox::SandboxProfile,
    session::{Session, SessionKind},
};
use willie_proto::session::{
    CreateParams, CreateResult, GitIdentity, IdParams as SessionIdParams,
    RenameParams as SessionRenameParams, SessionList, method as session,
};
use willie_proto::tool::{
    InstallParams, ToolList, UpdateParams, method as tool,
};
use willie_proto::{
    daemon::DoctorReport,
    job::method as job,
    plugin::{EnableParams, PluginStatus, method as plugin},
    project::{
        AddParams, AddResult, IdParams, JobRef, ProjectList, ReadFileParams,
        ReadFileResult, RelocateParams, RemoveParams, RenameParams,
        SetSandboxParams, TreeParams, TreeResult, method as project,
    },
    rpc::Notification,
    state::{Snapshot, method as state},
    usage::{UsageSnapshot, method as usage},
};

use crate::{
    config::{EngineConfig, Projects, Ui},
    daemon::{DaemonState, DaemonSupervisor},
    discover::{self, Candidate},
    distro::{DistroManager, DistroStatus, locate_image},
    error::{EngineError, WslError},
    paths,
    prereqs::{WslStatus, wsl_status},
};

/// How deep [`Engine::discover_projects`] walks under each configured
/// root: enough to find `group/repo` layouts without wandering into
/// every checkout's own directory tree.
const DISCOVER_MAX_DEPTH: usize = 2;

/// The namespace [`Engine::plugin_call`] alone is allowed to forward.
/// Mirrors `willied`'s own `PLUGIN_METHOD_PREFIX`: the two crates never
/// depend on each other, so each names it on its own side of the wire.
const PLUGIN_METHOD_PREFIX: &str = "profile.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    pub code: String,
    pub message: String,
    pub remediation: String,
}

impl From<&EngineError> for Problem {
    fn from(err: &EngineError) -> Self {
        Self {
            code: err.code().to_owned(),
            message: err.to_string(),
            remediation: err.remediation(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionOpened {
    pub session: Session,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_problem: Option<Problem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineStatus {
    pub engine_version: String,
    pub wsl: WslStatus,
    pub distro: Option<DistroStatus>,
    pub distro_error: Option<Problem>,
    pub daemon: DaemonState,
    pub doctor: Option<DoctorReport>,
    pub image_available: bool,
}

#[derive(Debug)]
pub struct Engine {
    image_candidates: Vec<PathBuf>,
    distro: DistroManager,
    daemon: DaemonSupervisor,
    last_doctor: Option<DoctorReport>,
    #[cfg(windows)]
    pub(crate) embedded: Option<crate::embed::imp::Embedded>,
}

impl Engine {
    #[must_use]
    pub fn new(image_candidates: Vec<PathBuf>) -> Self {
        Self {
            image_candidates,
            distro: DistroManager,
            daemon: DaemonSupervisor::new(),
            last_doctor: None,
            #[cfg(windows)]
            embedded: None,
        }
    }

    pub fn status(&mut self) -> EngineStatus {
        let (distro, distro_error) = match self.distro.status() {
            Ok(s) => (Some(s), None),
            Err(e) => (None, Some(Problem::from(&EngineError::Wsl(e)))),
        };
        EngineStatus {
            engine_version: willie_core::VERSION.to_owned(),
            wsl: wsl_status(),
            distro,
            distro_error,
            daemon: self.daemon.state(),
            doctor: self.last_doctor.clone(),
            image_available: locate_image(&self.image_candidates).is_some(),
        }
    }

    pub fn install_distro(&mut self) -> Result<(), EngineError> {
        let image = locate_image(&self.image_candidates)
            .ok_or(EngineError::ImageNotFound)?;
        self.daemon.stop()?;
        self.distro.install(&image)?;
        self.last_doctor = None;
        Ok(())
    }

    /// The engine drives exactly one distribution. Without it `wsl.exe`
    /// answers with its own message and a code the user cannot act on,
    /// so the missing registration is reported before the spawn.
    fn ensure_distro_registered(&self) -> Result<(), EngineError> {
        if self.distro.status()?.registered {
            Ok(())
        } else {
            Err(EngineError::DistroNotRegistered)
        }
    }

    pub fn start_daemon(&mut self) -> Result<(), EngineError> {
        self.ensure_distro_registered()?;
        self.daemon.start().map(drop)
    }

    pub fn stop_daemon(&mut self) -> Result<(), EngineError> {
        self.daemon.stop()
    }

    pub fn run_doctor(&mut self) -> Result<DoctorReport, EngineError> {
        if !matches!(self.daemon.state(), DaemonState::Running { .. }) {
            self.ensure_distro_registered()?;
            self.daemon.start()?;
        }
        let report = match self.daemon.doctor() {
            Ok(report) => report,
            // A well-formed error reply proves the daemon is alive.
            Err(err @ EngineError::Rpc(_)) => return Err(err),
            // Anything else: the supervisor has already reaped a dead or
            // unresponsive daemon. One restart is the supervision
            // promise; a second failure is the user's to see.
            Err(_) => {
                self.ensure_distro_registered()?;
                self.daemon.start()?;
                self.daemon.doctor()?
            }
        };
        self.last_doctor = Some(report.clone());
        Ok(report)
    }

    /// Distro pre-flight, then a call the daemon supervisor forwards
    /// over RPC, starting the daemon on demand. Every project and job
    /// method is a thin wrapper around this. The pre-flight is skipped
    /// once a daemon is already connected — same rule [`Engine::run_doctor`]
    /// already applies — which is also what lets a test drive a call
    /// against an injected connection with no `wsl.exe` in the loop.
    fn daemon_call<P: Serialize, R: DeserializeOwned>(
        &mut self,
        method: &str,
        params: P,
    ) -> Result<R, EngineError> {
        if !matches!(self.daemon.state(), DaemonState::Running { .. }) {
            self.ensure_distro_registered()?;
        }
        self.daemon.call(method, params)
    }

    pub fn project_list(&mut self) -> Result<ProjectList, EngineError> {
        self.daemon_call(project::LIST, serde_json::json!({}))
    }

    /// Only checks that the Windows path exists and is a directory;
    /// mapping it into the distro (`/mnt/c/...`) is the daemon's job.
    pub fn project_add(
        &mut self,
        windows_path: &str,
        name: Option<String>,
    ) -> Result<AddResult, EngineError> {
        if !std::path::Path::new(windows_path).is_dir() {
            return Err(EngineError::PathNotFound {
                path: windows_path.to_owned(),
            });
        }
        self.daemon_call(
            project::ADD,
            AddParams {
                windows_path: windows_path.to_owned(),
                name,
            },
        )
    }

    pub fn project_remove(
        &mut self,
        id: ProjectId,
        delete_workspace: bool,
        force: bool,
    ) -> Result<JobRef, EngineError> {
        self.daemon_call(
            project::REMOVE,
            RemoveParams {
                id,
                delete_workspace,
                force,
            },
        )
    }

    pub fn project_sync_to_windows(
        &mut self,
        id: ProjectId,
    ) -> Result<JobRef, EngineError> {
        self.daemon_call(project::SYNC_TO_WINDOWS, IdParams { id })
    }

    pub fn project_update_from_windows(
        &mut self,
        id: ProjectId,
    ) -> Result<JobRef, EngineError> {
        self.daemon_call(project::UPDATE_FROM_WINDOWS, IdParams { id })
    }

    pub fn project_relocate(
        &mut self,
        id: ProjectId,
        windows_path: String,
    ) -> Result<JobRef, EngineError> {
        self.daemon_call(project::RELOCATE, RelocateParams { id, windows_path })
    }

    pub fn project_rename(
        &mut self,
        id: ProjectId,
        name: String,
    ) -> Result<Project, EngineError> {
        self.daemon_call(project::RENAME, RenameParams { id, name })
    }

    pub fn project_set_sandbox(
        &mut self,
        id: ProjectId,
        profile: SandboxProfile,
    ) -> Result<Project, EngineError> {
        self.daemon_call(
            project::SET_SANDBOX,
            SetSandboxParams {
                project_id: id,
                profile,
            },
        )
    }

    /// Lists a workspace directory (its root when `path` is absent), with
    /// each entry's aggregated git status and the branch, for the
    /// workspace tree.
    pub fn project_tree(
        &mut self,
        id: ProjectId,
        path: Option<String>,
    ) -> Result<TreeResult, EngineError> {
        self.daemon_call(project::TREE, TreeParams { id, path })
    }

    /// Reads one workspace file for the read-only preview.
    pub fn project_read_file(
        &mut self,
        id: ProjectId,
        path: String,
    ) -> Result<ReadFileResult, EngineError> {
        self.daemon_call(project::READ_FILE, ReadFileParams { id, path })
    }

    pub fn session_create(
        &mut self,
        project_id: ProjectId,
        git_identity: Option<GitIdentity>,
        resume: bool,
        resume_from: Option<SessionId>,
        kind: SessionKind,
    ) -> Result<CreateResult, EngineError> {
        self.daemon_call(
            session::CREATE,
            CreateParams {
                project_id,
                git_identity,
                resume,
                resume_from,
                kind,
            },
        )
    }

    /// Create a session and open its terminal. A terminal that fails to
    /// launch does NOT undo the session (it is alive and attachable) — it
    /// comes back as `terminal_problem` for the UI to surface. `kind`
    /// chooses an agent conversation or an interactive shell.
    pub fn session_open(
        &mut self,
        project_id: ProjectId,
        kind: SessionKind,
    ) -> Result<SessionOpened, EngineError> {
        self.session_open_impl(project_id, false, None, kind)
    }

    /// Continue a project's last conversation: create the session with
    /// `resume: true` and open its terminal, same shape as
    /// [`Engine::session_open`]. `resume_from` names the finished session
    /// to continue; absent, the daemon picks the project's most recent
    /// one. Always an agent session — resuming a shell has no meaning.
    pub fn session_resume(
        &mut self,
        project_id: ProjectId,
        resume_from: Option<SessionId>,
    ) -> Result<SessionOpened, EngineError> {
        self.session_open_impl(
            project_id,
            true,
            resume_from,
            SessionKind::Agent,
        )
    }

    /// Shared body for [`Engine::session_open`] and
    /// [`Engine::session_resume`]: they differ only in the `resume`,
    /// `resume_from` and `kind` passed to the daemon.
    fn session_open_impl(
        &mut self,
        project_id: ProjectId,
        resume: bool,
        resume_from: Option<SessionId>,
        kind: SessionKind,
    ) -> Result<SessionOpened, EngineError> {
        let identity = crate::identity::windows_git_identity();
        let created = self.session_create(
            project_id,
            identity,
            resume,
            resume_from,
            kind,
        )?;
        let title = self
            .project_title(project_id)
            .unwrap_or_else(|| created.session.id.to_string());
        let terminal_problem =
            match crate::terminal::open_tab(created.session.id, &title) {
                Ok(()) => None,
                Err(e) => Some(Problem::from(&EngineError::TerminalLaunch {
                    message: e.to_string(),
                    attach_hint: attach_hint(created.session.id),
                })),
            };
        Ok(SessionOpened {
            session: created.session,
            terminal_problem,
        })
    }

    /// Renames or clears (`label: None`) a session's display label.
    pub fn session_rename(
        &mut self,
        id: SessionId,
        label: Option<String>,
    ) -> Result<Session, EngineError> {
        self.daemon_call(session::RENAME, SessionRenameParams { id, label })
    }

    /// Open (another) terminal for an existing session.
    pub fn session_attach(
        &mut self,
        id: SessionId,
        title: String,
    ) -> Result<(), EngineError> {
        crate::terminal::open_tab(id, &title).map_err(|e| {
            EngineError::TerminalLaunch {
                message: e.to_string(),
                attach_hint: attach_hint(id),
            }
        })
    }

    /// The display name for a session's tab: the project's name if known.
    fn project_title(&mut self, id: ProjectId) -> Option<String> {
        self.project_list()
            .ok()?
            .projects
            .into_iter()
            .find(|p| p.id == id)
            .map(|p| p.name)
    }

    pub fn session_stop(&mut self, id: SessionId) -> Result<(), EngineError> {
        self.daemon_call(session::STOP, SessionIdParams { id })
    }

    pub fn session_list(&mut self) -> Result<SessionList, EngineError> {
        self.daemon_call(session::LIST, serde_json::json!({}))
    }

    pub fn tool_install(
        &mut self,
        harness: &str,
    ) -> Result<JobRef, EngineError> {
        self.daemon_call(
            tool::INSTALL,
            InstallParams {
                harness: harness.to_owned(),
            },
        )
    }

    pub fn tool_list(&mut self) -> Result<ToolList, EngineError> {
        self.daemon_call(tool::LIST, serde_json::json!({}))
    }

    pub fn tool_update(&mut self, tool: &str) -> Result<JobRef, EngineError> {
        self.daemon_call(
            tool::UPDATE,
            UpdateParams {
                tool: tool.to_owned(),
            },
        )
    }

    pub fn job_cancel(&mut self, id: JobId) -> Result<(), EngineError> {
        self.daemon_call(job::CANCEL, serde_json::json!({ "id": id }))
    }

    pub fn plugin_list(&mut self) -> Result<Vec<PluginStatus>, EngineError> {
        self.daemon_call(plugin::LIST, serde_json::json!({}))
    }

    /// Sends empty params — the daemon fills in the current sessions and
    /// projects itself, the engine only relays the request.
    pub fn usage_snapshot(&mut self) -> Result<UsageSnapshot, EngineError> {
        self.daemon_call(usage::SNAPSHOT, serde_json::json!({}))
    }

    /// `project_id: None` enables the plugin globally; `Some` enables it
    /// for that project only. Only meaningful for a `PerProject`-scoped
    /// plugin — the daemon refuses the scope mismatch otherwise.
    pub fn plugin_enable(
        &mut self,
        id: String,
        project_id: Option<ProjectId>,
    ) -> Result<PluginStatus, EngineError> {
        self.daemon_call(plugin::ENABLE, EnableParams { id, project_id })
    }

    pub fn plugin_disable(
        &mut self,
        id: String,
        project_id: Option<ProjectId>,
    ) -> Result<PluginStatus, EngineError> {
        self.daemon_call(plugin::DISABLE, EnableParams { id, project_id })
    }

    /// The one seam the webview reaches a plugin's own methods through
    /// (`profile.list`, `profile.apply`, …) — a security boundary: without
    /// it the webview could invoke `daemon.shutdown`/`project.remove` too.
    pub fn plugin_call(
        &mut self,
        method: String,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, EngineError> {
        if !method.starts_with(PLUGIN_METHOD_PREFIX) {
            return Err(EngineError::MethodNotServed { method });
        }
        self.daemon_call(&method, params)
    }

    pub fn state_snapshot(&mut self) -> Result<Snapshot, EngineError> {
        self.daemon_call(state::SNAPSHOT, serde_json::json!({}))
    }

    /// Roots configured in `engine.toml`, empty when the file is absent.
    #[must_use]
    pub fn projects_roots(&self) -> Vec<String> {
        paths::engine_toml_path()
            .map(|p| EngineConfig::load(&p).projects.roots)
            .unwrap_or_default()
    }

    /// Persists the discovery roots to `engine.toml`, creating the data
    /// directory the first time a root is set. Load-modify-save so an
    /// already-saved `[ui]` table is kept, not clobbered.
    pub fn set_projects_roots(
        &self,
        roots: Vec<String>,
    ) -> Result<(), EngineError> {
        let path =
            paths::engine_toml_path().ok_or_else(|| WslError::Unparseable {
                what: "LOCALAPPDATA",
                text: String::new(),
            })?;
        let mut config = EngineConfig::load(&path);
        config.projects = Projects { roots };
        config.save(&path).map_err(|e| EngineError::ConfigWrite {
            path: path.display().to_string(),
            message: e.to_string(),
        })
    }

    /// The UI preferences saved in `engine.toml`'s `[ui]` table, empty
    /// when the file or the table itself is absent.
    #[must_use]
    pub fn ui_prefs(&self) -> Ui {
        paths::engine_toml_path()
            .map(|p| EngineConfig::load(&p).ui)
            .unwrap_or_default()
    }

    /// Persists the UI preferences. Load-modify-save so the `[projects]`
    /// table already on disk is kept, not clobbered.
    pub fn set_ui_prefs(&self, ui: Ui) -> Result<(), EngineError> {
        let path =
            paths::engine_toml_path().ok_or_else(|| WslError::Unparseable {
                what: "LOCALAPPDATA",
                text: String::new(),
            })?;
        let mut config = EngineConfig::load(&path);
        config.ui = ui;
        config.save(&path).map_err(|e| EngineError::ConfigWrite {
            path: path.display().to_string(),
            message: e.to_string(),
        })
    }

    /// Repositories found under the configured roots. Pure filesystem
    /// work, host-side; the result still needs `project_add` to be
    /// registered with the daemon.
    pub fn discover_projects(&self) -> Result<Vec<Candidate>, EngineError> {
        Ok(discover::discover(
            &self.projects_roots(),
            DISCOVER_MAX_DEPTH,
        ))
    }

    /// A live subscription to the daemon's notification stream, or
    /// `None` when no daemon is currently running.
    #[must_use]
    pub fn subscribe_events(&self) -> Option<Receiver<Notification>> {
        self.daemon.subscribe()
    }
}

/// The command a user runs to reach a session directly, quoted in
/// `terminal_launch_failed`'s remediation when no tab could be opened.
fn attach_hint(id: SessionId) -> String {
    format!(
        "wsl -d {} --user willie -- {} attach {id}",
        crate::wsl::DISTRO_NAME,
        crate::terminal::WILLIE_BIN,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problems_carry_code_message_and_remediation() {
        let p = Problem::from(&EngineError::Timeout {
            method: "daemon.health".into(),
        });
        assert_eq!(p.code, "daemon_timeout");
        assert!(p.message.contains("daemon.health"));
        assert!(!p.remediation.is_empty());
    }

    #[test]
    fn status_serialises_with_snake_case_daemon_state() {
        let status = EngineStatus {
            engine_version: "0.1.0".into(),
            wsl: WslStatus {
                installed: false,
                version: None,
                meets_minimum: false,
                minimum: "2.4.4".into(),
            },
            distro: None,
            distro_error: None,
            daemon: DaemonState::Stopped,
            doctor: None,
            image_available: false,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["daemon"]["state"], "stopped");
        assert_eq!(json["wsl"]["minimum"], "2.4.4");
    }

    #[test]
    fn project_add_rejects_a_path_that_does_not_exist() {
        let mut engine = Engine::new(Vec::new());
        let err = engine
            .project_add(r"C:\does\not\exist\willie-xyz", None)
            .unwrap_err();
        assert_eq!(err.code(), "path_not_found");
    }

    /// The security boundary itself: a method outside the `profile.`
    /// namespace is refused before `plugin_call` ever reaches the
    /// daemon — no distro, no running daemon needed to observe it.
    #[test]
    fn plugin_call_refuses_a_method_outside_the_profile_namespace() {
        let mut engine = Engine::new(Vec::new());
        let err = engine
            .plugin_call("daemon.shutdown".into(), serde_json::json!({}))
            .unwrap_err();
        assert_eq!(err.code(), "method_not_served");
    }

    /// A method that merely starts with the right prefix as a
    /// substring, but not at the start, is not "profile.*" either.
    #[test]
    fn plugin_call_refuses_a_method_that_only_contains_the_prefix() {
        let mut engine = Engine::new(Vec::new());
        let err = engine
            .plugin_call("plugin.profile.list".into(), serde_json::json!({}))
            .unwrap_err();
        assert_eq!(err.code(), "method_not_served");
    }

    /// A peer that echoes the `session.create` request's `kind` and
    /// `resume_from` back inside a minimal `CreateResult`, so a test can
    /// assert on the exact `CreateParams` `Engine::session_create` put on
    /// the wire without a running daemon.
    fn create_echo_peer_main() {
        use std::io::{BufRead, Write};

        crate::test_support::announce_ready();
        let stdin = std::io::stdin();
        let mut out = std::io::stdout();
        for line in stdin.lock().lines().map_while(Result::ok) {
            let req: willie_proto::rpc::Request =
                serde_json::from_str(&line).unwrap();
            let params: CreateParams =
                serde_json::from_value(req.params.clone()).unwrap();
            let result = serde_json::json!({
                "session": {
                    "id": SessionId::new().to_string(),
                    "project_id": params.project_id.to_string(),
                    "harness": "claude-code",
                    "workspace": "/home/willie/projects/x",
                    "kind": params.kind,
                    "state": { "state": "creating" },
                    "created_at": "0",
                    "resumed_from": params.resume_from,
                },
            });
            let resp = willie_proto::rpc::Response::ok(req.id, result).unwrap();
            writeln!(out, "{}", serde_json::to_string(&resp).unwrap()).unwrap();
            out.flush().unwrap();
        }
        std::process::exit(0);
    }

    /// `session_open`/`session_resume` both narrow to `session_create`,
    /// the seam that actually builds `CreateParams` and calls the
    /// daemon — tested here directly rather than through the public
    /// wrappers, which also open a real terminal tab that a unit test
    /// must not spawn. A scripted peer stands in for `willied`, wrapped
    /// as a "connected" daemon so no `wsl.exe` is involved.
    #[test]
    fn session_create_sends_the_requested_kind_and_resume_from() {
        if std::env::var_os("WILLIE_CREATE_ECHO_MODE").is_some() {
            create_echo_peer_main();
            return;
        }
        let child = crate::test_support::spawn_peer(
            "engine::tests::session_create_sends_the_requested_kind_and_resume_from",
            "WILLIE_CREATE_ECHO_MODE",
        );
        let mut process =
            crate::process::WslProcess::from_child(child).unwrap();
        let mut transport = process.transport().unwrap();
        crate::test_support::await_ready(&mut transport);
        let client = crate::rpc::RpcClient::new(
            transport,
            std::time::Duration::from_secs(5),
        );
        let mut engine = Engine {
            image_candidates: Vec::new(),
            distro: DistroManager,
            daemon: DaemonSupervisor::connected(process, client),
            last_doctor: None,
            #[cfg(windows)]
            embedded: None,
        };

        let project_id = ProjectId::new();
        let created = engine
            .session_create(project_id, None, false, None, SessionKind::Shell)
            .expect("session.create (shell)");
        assert_eq!(created.session.kind, SessionKind::Shell);
        assert!(created.session.resumed_from.is_none());

        let resume_from = SessionId::new();
        let created = engine
            .session_create(
                project_id,
                None,
                true,
                Some(resume_from),
                SessionKind::Agent,
            )
            .expect("session.create (resume)");
        assert_eq!(created.session.resumed_from, Some(resume_from));
    }
}
