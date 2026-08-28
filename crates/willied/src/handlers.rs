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
use willie_proto::{
    PROTOCOL_VERSION,
    daemon::{DoctorReport, Health, Hello, HelloReply},
    job::Job,
    project::{
        self, AddParams, ProjectList, RelocateParams, RemoveParams,
        RenameParams,
    },
    rpc::RpcError,
    state::Snapshot,
    tool::InstallParams,
};

use crate::{projects::Ops, sessions::SessionOps, state::State};

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

/// Recovers a poisoned lock instead of panicking: one worker's panic must
/// not take a reader's snapshot down with it.
fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
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

/// Recomputes each project's `source_present` from the filesystem, then
/// answers with the fresh snapshot.
pub fn state_snapshot(
    ops: &Ops,
    state: &Mutex<State>,
) -> Result<Value, RpcError> {
    ops.refresh_source_present();
    let snapshot: Snapshot = lock(state).snapshot();
    serde_json::to_value(snapshot).map_err(internal)
}
