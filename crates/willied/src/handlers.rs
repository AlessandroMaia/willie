//! Request handlers. `daemon.*` stay pure; `project.*`, `job.*`, `tool.*`
//! and `state.*` reach into the shared `State` and the project `Ops`. Each
//! returns the JSON result or a coded error the server turns into a
//! `Response`.

use std::{
    sync::{Mutex, MutexGuard, PoisonError},
    time::Instant,
};

use serde_json::Value;
use willie_core::id::JobId;
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
}
