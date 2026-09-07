//! Request handlers. `daemon.*` stay pure; `project.*`, `job.*`, `tool.*`
//! and `state.*` reach into the shared `State` and the project `Ops`. Each
//! returns the JSON result or a coded error the server turns into a
//! `Response`.

use std::{
    sync::{Mutex, MutexGuard, PoisonError},
    time::Instant,
};

use serde_json::Value;
use willie_core::id::{JobId, ProjectId};
use willie_harness::Harness;
use willie_proto::{
    PROTOCOL_VERSION,
    daemon::{DoctorReport, Health, Hello, HelloReply},
    job::Job,
    project::{
        self, AddParams, ProjectList, RelocateParams, RemoveParams,
        RenameParams, SetSandboxParams,
    },
    rpc::RpcError,
    state::Snapshot,
    tool::{InstallParams, UpdateParams},
};

use crate::{
    plugins::PluginHost, projects::Ops, sessions::SessionOps, state::State,
};

fn internal(e: impl std::fmt::Display) -> RpcError {
    RpcError::new("internal_error", e.to_string())
}

fn invalid_params(e: impl std::fmt::Display) -> RpcError {
    RpcError::new("invalid_params", e.to_string())
}

/// Maps a project `OpError` onto the wire error, preserving its code and
/// remediation. One place so every project method reports the same shape.
fn op_error(e: crate::projects::OpError) -> RpcError {
    RpcError::new(&e.code, e.message).with_remediation(e.remediation)
}

/// Maps a `CapabilityError` onto the wire error, the same two codes the
/// session-create path reports for the same two refusals. Goes through
/// `OpError` rather than building the payload again: the daemon has one
/// conversion out of a sandbox refusal, and `op_error` is already the
/// one place an `OpError` becomes a reply.
fn capability_error(e: willie_core::sandbox::CapabilityError) -> RpcError {
    op_error(e.into())
}

/// Recovers a poisoned lock instead of panicking: one worker's panic must
/// not take a reader's snapshot down with it.
fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The same poison-recovering lock for the plugin host: a plugin's panic is
/// already caught at the host's own boundary, so the mutex is never
/// poisoned by one, but recover anyway rather than risk a panic here.
fn lock_host(host: &Mutex<PluginHost>) -> MutexGuard<'_, PluginHost> {
    host.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Extracts the `id` of a `{ "id": <JobId> }` params object. Avoids a
/// one-field params struct (and a `serde` derive) for the job methods.
fn job_id(p: &Value) -> Result<JobId, RpcError> {
    match p.get("id") {
        Some(v) => serde_json::from_value(v.clone()).map_err(invalid_params),
        None => Err(RpcError::new("invalid_params", "missing field `id`")),
    }
}

pub fn hello(params_in: Value) -> Result<Value, RpcError> {
    let hello: Hello = serde_json::from_value(params_in)
        .map_err(|e| RpcError::new("invalid_params", format!("hello: {e}")))?;
    if hello.protocol_version != PROTOCOL_VERSION {
        return Err(RpcError::new(
            "protocol_version_mismatch",
            format!(
                "client speaks protocol {}, daemon speaks {PROTOCOL_VERSION}",
                hello.protocol_version
            ),
        )
        .with_remediation(
            "update Willie so engine and daemon share a version",
        ));
    }
    let reply = HelloReply {
        willie_version: willie_core::VERSION.to_owned(),
        protocol_version: PROTOCOL_VERSION,
        distro_image_version: willie_linux::paths::image_version(),
    };
    serde_json::to_value(reply).map_err(internal)
}

pub fn health(started: Instant) -> Result<Value, RpcError> {
    let health = Health {
        pid: std::process::id(),
        uptime_secs: started.elapsed().as_secs(),
        willie_version: willie_core::VERSION.to_owned(),
    };
    serde_json::to_value(health).map_err(internal)
}

pub fn doctor(run: fn() -> DoctorReport) -> Result<Value, RpcError> {
    #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
    let mut report = run();
    #[cfg(target_os = "linux")]
    report
        .checks
        .push(crate::harness::doctor_check(&crate::harness::home()));
    serde_json::to_value(report).map_err(internal)
}

pub fn project_add(ops: &Ops, p: Value) -> Result<Value, RpcError> {
    let p: AddParams = serde_json::from_value(p).map_err(invalid_params)?;
    let res = ops.add(p).map_err(op_error)?;
    serde_json::to_value(res).map_err(internal)
}

pub fn project_remove(ops: &Ops, p: Value) -> Result<Value, RpcError> {
    let RemoveParams {
        id,
        delete_workspace,
        force,
    } = serde_json::from_value(p).map_err(invalid_params)?;
    let res = ops.remove(id, delete_workspace, force).map_err(op_error)?;
    serde_json::to_value(res).map_err(internal)
}

pub fn project_sync(ops: &Ops, p: Value) -> Result<Value, RpcError> {
    let project::IdParams { id } =
        serde_json::from_value(p).map_err(invalid_params)?;
    let res = ops.sync_to_windows(id).map_err(op_error)?;
    serde_json::to_value(res).map_err(internal)
}

pub fn project_update(ops: &Ops, p: Value) -> Result<Value, RpcError> {
    let project::IdParams { id } =
        serde_json::from_value(p).map_err(invalid_params)?;
    let res = ops.update_from_windows(id).map_err(op_error)?;
    serde_json::to_value(res).map_err(internal)
}

pub fn project_relocate(ops: &Ops, p: Value) -> Result<Value, RpcError> {
    let p: RelocateParams =
        serde_json::from_value(p).map_err(invalid_params)?;
    let res = ops.relocate(p).map_err(op_error)?;
    serde_json::to_value(res).map_err(internal)
}

pub fn project_rename(ops: &Ops, p: Value) -> Result<Value, RpcError> {
    let p: RenameParams = serde_json::from_value(p).map_err(invalid_params)?;
    let res = ops.rename(p).map_err(op_error)?;
    serde_json::to_value(res).map_err(internal)
}

pub fn project_set_sandbox(ops: &Ops, p: Value) -> Result<Value, RpcError> {
    let p: SetSandboxParams =
        serde_json::from_value(p).map_err(invalid_params)?;
    let res = ops.set_sandbox(p).map_err(op_error)?;
    serde_json::to_value(res).map_err(internal)
}

pub fn project_list(state: &Mutex<State>) -> Result<Value, RpcError> {
    let projects = lock(state).projects.values().cloned().collect();
    serde_json::to_value(ProjectList { projects }).map_err(internal)
}

pub fn job_list(state: &Mutex<State>) -> Result<Value, RpcError> {
    let jobs: Vec<Job> = lock(state).jobs.values().cloned().collect();
    serde_json::to_value(serde_json::json!({ "jobs": jobs })).map_err(internal)
}

pub fn job_get(state: &Mutex<State>, p: Value) -> Result<Value, RpcError> {
    let id = job_id(&p)?;
    let job = lock(state).jobs.get(&id).cloned().ok_or_else(|| {
        RpcError::new("job_not_found", format!("no job with id `{id}`"))
            .with_remediation("check the job id against state.snapshot")
    })?;
    serde_json::to_value(job).map_err(internal)
}

pub fn job_cancel(ops: &Ops, p: Value) -> Result<Value, RpcError> {
    let id = job_id(&p)?;
    ops.cancel_job(&id);
    Ok(Value::Null)
}

pub fn tool_install(ops: &Ops, p: Value) -> Result<Value, RpcError> {
    let InstallParams { harness } =
        serde_json::from_value(p).map_err(invalid_params)?;
    let res = ops.install_tool(&harness).map_err(op_error)?;
    serde_json::to_value(res).map_err(internal)
}

pub fn tool_list(ops: &Ops) -> Result<Value, RpcError> {
    serde_json::to_value(ops.list_tools()).map_err(internal)
}

pub fn tool_update(ops: &Ops, p: Value) -> Result<Value, RpcError> {
    let UpdateParams { tool } =
        serde_json::from_value(p).map_err(invalid_params)?;
    let res = ops.update_tool(&tool).map_err(op_error)?;
    serde_json::to_value(res).map_err(internal)
}

pub fn session_create(ops: &SessionOps, p: Value) -> Result<Value, RpcError> {
    let params = serde_json::from_value(p).map_err(invalid_params)?;
    let session = ops.create(params).map_err(op_error)?;
    serde_json::to_value(willie_proto::session::CreateResult { session })
        .map_err(internal)
}

pub fn session_stop(ops: &SessionOps, p: Value) -> Result<Value, RpcError> {
    let willie_proto::session::IdParams { id } =
        serde_json::from_value(p).map_err(invalid_params)?;
    ops.stop(id).map_err(op_error)?;
    Ok(Value::Null)
}

pub fn session_rename(ops: &SessionOps, p: Value) -> Result<Value, RpcError> {
    let params: willie_proto::session::RenameParams =
        serde_json::from_value(p).map_err(invalid_params)?;
    let session = ops.rename(params).map_err(op_error)?;
    serde_json::to_value(session).map_err(internal)
}

pub fn session_list(ops: &SessionOps) -> Result<Value, RpcError> {
    serde_json::to_value(willie_proto::session::SessionList {
        sessions: ops.list(),
    })
    .map_err(internal)
}

/// What a session for this project would run under, without starting
/// one: the same two layers `session.create` resolves, reported row by
/// row. Answers `project_not_found` for an unknown id, the same code
/// every other project method uses.
pub fn sandbox_explain(
    state: &Mutex<State>,
    p: Value,
) -> Result<Value, RpcError> {
    let willie_proto::sandbox::ExplainParams { project_id } =
        serde_json::from_value(p).map_err(invalid_params)?;
    let project = lock(state)
        .projects
        .get(&project_id)
        .cloned()
        .ok_or_else(|| op_error(crate::projects::not_found_err(project_id)))?;
    if let Some(problem) = &project.sandbox_problem {
        return Err(op_error(crate::projects::OpError::from_problem(problem)));
    }
    let defaults = crate::harness::claude().default_capabilities();
    let home = crate::harness::home();
    let capabilities = willie_core::sandbox::resolve(
        defaults.clone(),
        &project.sandbox,
        &home.to_string_lossy(),
    )
    .map_err(capability_error)?;
    let entries = willie_core::sandbox::explain(
        defaults,
        &project.sandbox,
        &home.to_string_lossy(),
    )
    .map_err(capability_error)?;
    serde_json::to_value(willie_proto::sandbox::ExplainResult {
        entries,
        capabilities,
    })
    .map_err(internal)
}

/// The host's registry of plugins, each merged with its enablement and
/// degraded flag.
pub fn plugin_list(host: &Mutex<PluginHost>) -> Result<Value, RpcError> {
    let plugins = lock_host(host).list();
    serde_json::to_value(plugins).map_err(internal)
}

/// Enables a plugin. `EnableParams.project_id` picks the scope: absent is a
/// global enable, present a per-project one; a scope the plugin's manifest
/// forbids is refused with `plugin_scope_mismatch`.
pub fn plugin_enable(
    host: &Mutex<PluginHost>,
    p: Value,
) -> Result<Value, RpcError> {
    let params: willie_proto::plugin::EnableParams =
        serde_json::from_value(p).map_err(invalid_params)?;
    let status = lock_host(host).enable(params).map_err(op_error)?;
    serde_json::to_value(status).map_err(internal)
}

/// Disables a plugin in the scope its `EnableParams` implies.
pub fn plugin_disable(
    host: &Mutex<PluginHost>,
    p: Value,
) -> Result<Value, RpcError> {
    let params: willie_proto::plugin::EnableParams =
        serde_json::from_value(p).map_err(invalid_params)?;
    let status = lock_host(host).disable(params).map_err(op_error)?;
    serde_json::to_value(status).map_err(internal)
}

/// Routes an `<id>.<method>` call (e.g. `profile.list`) to its plugin. The
/// host resolves the leading id, refuses an unknown one with
/// `plugin_not_found` and a disabled one with `plugin_disabled`, and turns
/// a plugin panic into `plugin_panicked` rather than a daemon crash.
pub fn plugin_handle(
    host: &Mutex<PluginHost>,
    method: &str,
    p: Value,
) -> Result<Value, RpcError> {
    lock_host(host).handle(method, p).map_err(op_error)
}

/// The daemon-fills-targets seam (designs/plugins-and-profiles.md, "the
/// project is looked up through the daemon"): every `profile.*` call is
/// routed through here rather than straight to `plugin_handle`. A
/// `project_id` in the params is resolved to the project's ext4
/// `workspace` and the harness-state settings path *before* the plugin
/// ever runs, injected as `_workspace`/`_harness_settings` — the profiles
/// plugin reads those two fields and never reaches into daemon state
/// itself. Params carrying no `project_id` (`profile.list`,
/// `profile.create`, `profile.read_fragment`, `profile.write_fragment`)
/// pass through unchanged: not every `profile.*` method targets a
/// project. An unknown `project_id` answers `project_not_found` here,
/// before `plugin_handle` is reached at all, so it is never masked by a
/// `plugin_disabled` the plugin host would otherwise answer first.
pub fn profile_handle(
    state: &Mutex<State>,
    host: &Mutex<PluginHost>,
    method: &str,
    p: Value,
) -> Result<Value, RpcError> {
    let p = resolve_profile_targets(state, p)?;
    plugin_handle(host, method, p)
}

/// Looks up `params.project_id` (when present) in `state` and injects the
/// project's `workspace` and the harness-state settings path as
/// `_workspace`/`_harness_settings`. Any caller-supplied `_workspace`/
/// `_harness_settings` are stripped first, unconditionally: only the
/// daemon ever injects those two fields, so a `project_id`-less call
/// carries neither and the plugin's required `_workspace` then fails
/// closed instead of trusting a smuggled path.
fn resolve_profile_targets(
    state: &Mutex<State>,
    mut params: Value,
) -> Result<Value, RpcError> {
    if let Value::Object(map) = &mut params {
        map.remove("_workspace");
        map.remove("_harness_settings");
    }

    let Some(raw_id) = params.get("project_id").and_then(Value::as_str) else {
        return Ok(params);
    };
    let project_id: ProjectId = raw_id.parse().map_err(invalid_params)?;
    let project = lock(state)
        .projects
        .get(&project_id)
        .cloned()
        .ok_or_else(|| op_error(crate::projects::not_found_err(project_id)))?;

    let harness_settings = crate::harness::home()
        .join(".willie/agent-state/claude/dot-claude/settings.json");
    if let Value::Object(map) = &mut params {
        map.insert("_workspace".to_owned(), Value::String(project.workspace));
        map.insert(
            "_harness_settings".to_owned(),
            Value::String(harness_settings.to_string_lossy().into_owned()),
        );
    }
    Ok(params)
}

/// The daemon-fills-targets seam for `usage.*` (mirrors
/// `profile_handle`/`resolve_profile_targets`, with its own enrichment):
/// every call is routed through here rather than straight to
/// `plugin_handle`, so the usage plugin never reaches into daemon state
/// itself — it only ever sees what `enrich_usage_targets` injects.
pub fn usage_handle(
    state: &Mutex<State>,
    host: &Mutex<PluginHost>,
    method: &str,
    p: Value,
) -> Result<Value, RpcError> {
    let p = enrich_usage_targets(state, p);
    plugin_handle(host, method, p)
}

/// Strips any caller-supplied `_sessions`/`_home` (fail-closed, like
/// `resolve_profile_targets`: only the daemon fills those two fields),
/// then injects the daemon's own view of them: every session in `State`
/// as `{ id, project_id, workspace, window: [start, end|null] }` (session
/// timestamps are epoch-second strings; a still-live session's `end` is
/// `null`), and `_home` as the distro home directory. Best-effort: a
/// timestamp that fails to parse becomes a `0` start rather than dropping
/// the session.
fn enrich_usage_targets(state: &Mutex<State>, mut params: Value) -> Value {
    if !params.is_object() {
        params = Value::Object(serde_json::Map::new());
    }
    if let Value::Object(map) = &mut params {
        map.remove("_sessions");
        map.remove("_home");
    }

    let sessions: Vec<Value> = lock(state)
        .sessions
        .values()
        .map(|s| {
            let start = s.created_at.parse::<u64>().unwrap_or(0);
            let end =
                s.finished_at.as_deref().and_then(|t| t.parse::<u64>().ok());
            serde_json::json!({
                "id": s.id,
                "project_id": s.project_id,
                "workspace": s.workspace,
                "window": [start, end],
            })
        })
        .collect();

    if let Value::Object(map) = &mut params {
        map.insert("_sessions".to_owned(), Value::Array(sessions));
        map.insert(
            "_home".to_owned(),
            Value::String(
                crate::harness::home().to_string_lossy().into_owned(),
            ),
        );
    }
    params
}

/// Recomputes each project's `source_present` from the filesystem, then
/// answers with the fresh snapshot, its `plugins` field filled from the
/// host's live `list()`.
pub fn state_snapshot(
    ops: &Ops,
    state: &Mutex<State>,
    host: &Mutex<PluginHost>,
) -> Result<Value, RpcError> {
    ops.refresh_source_present();
    let mut snapshot: Snapshot = lock(state).snapshot();
    snapshot.plugins = lock_host(host).list();
    serde_json::to_value(snapshot).map_err(internal)
}

#[cfg(test)]
mod tests {
    use willie_core::{
        id::ProjectId,
        project::{Project, ProjectState, SandboxProblem},
        sandbox::SandboxProfile,
    };

    use super::*;

    fn project_with_a_sandbox_problem() -> Project {
        Project {
            id: ProjectId::new(),
            name: "p".into(),
            slug: "p".into(),
            source: "C:\\src".into(),
            workspace: "/w".into(),
            branch: "main".into(),
            state: ProjectState::Ready,
            source_present: true,
            created_at: "t".into(),
            sandbox: SandboxProfile::default(),
            sandbox_problem: Some(SandboxProblem {
                code: "sandbox_profile_invalid".into(),
                message: "the [sandbox] table could not be read: bad".into(),
                remediation: "fix the file".into(),
            }),
        }
    }

    /// A project whose `[sandbox]` table failed to parse must not
    /// answer with a resolved policy over the default profile it
    /// loaded with: that would silently hide the problem instead of
    /// surfacing it, so `sandbox.explain` refuses the same way
    /// `session.create` does.
    #[test]
    fn sandbox_explain_refuses_a_project_with_a_sandbox_problem() {
        let project = project_with_a_sandbox_problem();
        let pid = project.id;
        let state = Mutex::new(State::default());
        state.lock().unwrap().projects.insert(pid, project);

        let params =
            serde_json::to_value(willie_proto::sandbox::ExplainParams {
                project_id: pid,
            })
            .unwrap();
        let err = sandbox_explain(&state, params).unwrap_err();

        assert_eq!(err.code, "sandbox_profile_invalid");
        assert_eq!(err.remediation.as_deref(), Some("fix the file"));
    }

    /// A `profile.apply`-shaped call carrying a caller-supplied
    /// `_workspace` but no `project_id` must not reach the plugin with
    /// that workspace: only the daemon may inject `_workspace`/
    /// `_harness_settings`, so both are stripped before the (absent)
    /// `project_id` is even considered.
    #[test]
    fn resolve_profile_targets_strips_a_caller_supplied_workspace_without_project_id()
     {
        let state = Mutex::new(State::default());
        let params = serde_json::json!({
            "name": "x",
            "_workspace": "/any/existing/dir",
        });

        let resolved = resolve_profile_targets(&state, params).unwrap();

        assert!(resolved.get("_workspace").is_none());
        assert!(resolved.get("_harness_settings").is_none());
    }

    /// The normal path: a valid `project_id` still gets `_workspace`
    /// injected from the resolved project, so a genuine `profile.apply`
    /// against a project keeps working after the strip above.
    #[test]
    fn resolve_profile_targets_injects_workspace_from_a_valid_project_id() {
        let project = project_with_a_sandbox_problem();
        let pid = project.id;
        let workspace = project.workspace.clone();
        let state = Mutex::new(State::default());
        state.lock().unwrap().projects.insert(pid, project);

        let params = serde_json::json!({
            "name": "x",
            "project_id": pid.to_string(),
        });

        let resolved = resolve_profile_targets(&state, params).unwrap();

        assert_eq!(
            resolved.get("_workspace").and_then(Value::as_str),
            Some(workspace.as_str())
        );
        assert!(resolved.get("_harness_settings").is_some());
    }

    use willie_core::{
        id::SessionId,
        session::{Session, SessionKind, SessionState},
    };

    fn session(
        id: SessionId,
        project_id: ProjectId,
        created_at: &str,
    ) -> Session {
        Session {
            id,
            project_id,
            harness: "claude-code".into(),
            workspace: "/w".into(),
            kind: SessionKind::Agent,
            state: SessionState::Running,
            created_at: created_at.into(),
            started_at: None,
            finished_at: None,
            pid: None,
            clients: 0,
            resumed_from: None,
            label: None,
            title: None,
            sandbox: Default::default(),
        }
    }

    /// A host with the usage plugin enabled globally, its state under a
    /// scratch dir private to the caller (so no test's `enabled.toml`
    /// leaks into another's, the same reasoning `server.rs`'s
    /// `test_server` gives).
    fn usage_enabled_host() -> (Mutex<PluginHost>, std::path::PathBuf) {
        let dir = std::env::temp_dir()
            .join("willie-handlers-test-plugins")
            .join(ProjectId::new().to_string());
        let mut host = PluginHost::new(&dir);
        host.enable(willie_proto::plugin::EnableParams {
            id: "usage".into(),
            project_id: None,
        })
        .unwrap();
        (Mutex::new(host), dir)
    }

    /// A caller-supplied `_sessions`/`_home` must never reach the plugin:
    /// only the daemon fills those two fields, mirroring
    /// `resolve_profile_targets_strips_a_caller_supplied_workspace_without_project_id`.
    /// The real session in `State` is what comes back, not the smuggled one.
    #[test]
    fn enrich_usage_targets_strips_caller_supplied_sessions_and_home() {
        let state = Mutex::new(State::default());
        let sid = SessionId::new();
        let pid = ProjectId::new();
        state
            .lock()
            .unwrap()
            .sessions
            .insert(sid, session(sid, pid, "100"));

        let params = serde_json::json!({
            "_sessions": [{"id": "smuggled"}],
            "_home": "/some/smuggled/path",
        });

        let enriched = enrich_usage_targets(&state, params);

        let sessions =
            enriched.get("_sessions").and_then(Value::as_array).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].get("id").and_then(Value::as_str),
            Some(sid.to_string().as_str())
        );
        assert_eq!(
            enriched.get("_home").and_then(Value::as_str),
            Some(
                crate::harness::home()
                    .to_string_lossy()
                    .into_owned()
                    .as_str()
            )
        );
    }

    /// A still-live session (`finished_at: None`) enriches to a `window`
    /// whose end is JSON `null`, and its `created_at` epoch-seconds string
    /// parses into the start.
    #[test]
    fn enrich_usage_targets_reports_an_open_window_for_a_live_session() {
        let state = Mutex::new(State::default());
        let sid = SessionId::new();
        let pid = ProjectId::new();
        state
            .lock()
            .unwrap()
            .sessions
            .insert(sid, session(sid, pid, "42"));

        let enriched = enrich_usage_targets(&state, Value::Null);

        let entry = &enriched.get("_sessions").unwrap().as_array().unwrap()[0];
        assert_eq!(entry["project_id"], serde_json::json!(pid));
        assert_eq!(entry["window"], serde_json::json!([42, null]));
    }

    /// `usage.snapshot`, routed through `usage_handle` with two known
    /// sessions in `State`, comes back naming both — proof the routing and
    /// enrichment reach the plugin (no real JSONL log is planted; each
    /// session is simply present with zero tokens, same as the plugin's own
    /// "missing log dir" case).
    #[test]
    fn usage_snapshot_names_every_known_session() {
        let state = Mutex::new(State::default());
        let (id1, id2) = (SessionId::new(), SessionId::new());
        {
            let mut guard = state.lock().unwrap();
            guard
                .sessions
                .insert(id1, session(id1, ProjectId::new(), "10"));
            guard
                .sessions
                .insert(id2, session(id2, ProjectId::new(), "20"));
        }
        let (host, dir) = usage_enabled_host();

        let result = usage_handle(
            &state,
            &host,
            willie_proto::usage::method::SNAPSHOT,
            Value::Null,
        )
        .unwrap();
        let snapshot: willie_proto::usage::UsageSnapshot =
            serde_json::from_value(result).unwrap();

        let ids: Vec<SessionId> =
            snapshot.sessions.iter().map(|s| s.id).collect();
        assert!(ids.contains(&id1), "{ids:?}");
        assert!(ids.contains(&id2), "{ids:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
