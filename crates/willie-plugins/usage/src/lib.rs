//! Usage, cost and context tracking.
//!
//! `usage.snapshot` projects every daemon-supplied session's token usage
//! from its harness's on-disk log — best-effort throughout, so a missing
//! or unreadable log never fails the call, only leaves that session with
//! no usage data. `on_event` reacts to a session starting or exiting by
//! emitting a no-id signal the client re-pulls a snapshot on.

use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;
use serde_json::Value;
use willie_core::id::{ProjectId, SessionId};
use willie_harness::Harness;
use willie_plugin_api::{
    CoreEvent, Plugin, PluginCtx, PluginError, PluginManifest, PluginRequest,
    PluginResponse, Scope,
};
use willie_proto::usage::{ProjectUsage, SessionUsage, UsageSnapshot, method};

pub mod read;

/// The usage plugin.
#[derive(Debug, Clone, Copy, Default)]
pub struct UsagePlugin;

/// Constructs the plugin for the daemon's registry.
#[must_use]
pub fn plugin() -> UsagePlugin {
    UsagePlugin
}

impl Plugin for UsagePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "usage",
            name: "Usage, cost and context",
            scope: Scope::Global,
        }
    }

    fn handle(
        &mut self,
        _ctx: &PluginCtx<'_>,
        req: PluginRequest,
    ) -> Result<PluginResponse, PluginError> {
        match req.method.as_str() {
            method::SNAPSHOT => snapshot(req.params),
            other => Err(PluginError::BadRequest(format!(
                "unknown method `{other}`"
            ))),
        }
    }

    /// The design's `usage.updated` is a no-id signal (the client
    /// re-pulls `usage.snapshot`), not a rebuilt snapshot: `on_event`
    /// carries no session workspace/window, so there is nothing to
    /// project here.
    fn on_event(&mut self, ctx: &PluginCtx<'_>, ev: &CoreEvent) {
        match ev {
            CoreEvent::SessionStarted { .. }
            | CoreEvent::SessionExited { .. } => {
                ctx.emit("usage.updated", serde_json::json!({}));
            }
            CoreEvent::ProjectRegistered { .. } | CoreEvent::Tick => {}
        }
    }
}

// --------------------------------------------------------------- params

/// One daemon-filled `_sessions` entry. `window`'s second element is
/// `null` for a still-live session, resolved to "now" before it reaches
/// [`read_session`].
#[derive(Debug, Deserialize)]
struct SessionParam {
    id: SessionId,
    project_id: ProjectId,
    workspace: String,
    window: (u64, Option<u64>),
}

/// `usage.snapshot`'s params: entirely daemon-filled (mirrors
/// `profile.check`'s `_workspace`), never supplied by a caller directly.
/// Both fields default on a missing or malformed shape instead of
/// refusing the call — `usage.snapshot` is best-effort end to end.
#[derive(Debug, Default, Deserialize)]
struct SnapshotParams {
    #[serde(rename = "_home", default)]
    home: String,
    #[serde(rename = "_sessions", default)]
    sessions: Vec<SessionParam>,
}

// -------------------------------------------------------------- methods

/// `usage.snapshot`: reads each session's harness log, sums tokens per
/// project and returns the result as of now. A bad params shape simply
/// yields no sessions; each session's own read is best-effort.
fn snapshot(params: Value) -> Result<PluginResponse, PluginError> {
    let SnapshotParams { home, sessions } =
        serde_json::from_value(params).unwrap_or_default();
    let home = PathBuf::from(home);
    let now = now_secs();

    let mut usages = Vec::with_capacity(sessions.len());
    let mut totals: BTreeMap<ProjectId, u64> = BTreeMap::new();
    for session in &sessions {
        let window = (session.window.0, session.window.1.unwrap_or(now));
        let reading = read_session(&home, &session.workspace, window);
        let entry = totals.entry(session.project_id).or_insert(0);
        *entry = entry.saturating_add(reading.tokens);
        usages.push(SessionUsage {
            id: session.id,
            tokens: reading.tokens,
            context_pct: reading.context_pct,
        });
    }
    let projects = totals
        .into_iter()
        .map(|(id, tokens)| ProjectUsage { id, tokens })
        .collect();

    json_response(&UsageSnapshot {
        providers: Vec::new(),
        sessions: usages,
        projects,
        fetched_at: now.to_string(),
    })
}

// --------------------------------------------------------------- reads

/// Bytes of a session log's tail read from disk: enough to almost
/// always hold the newest usage-carrying line without loading a log
/// that can grow unbounded across a long session.
const TAIL_BYTES: u64 = 64 * 1024;

/// One session's token usage, resolved from its harness's on-disk log.
/// A harness with no logs directory, a missing log directory, an empty
/// listing or no file matching `window` all resolve to the same "no
/// usage data yet" reading, never a fault.
fn read_session(
    home: &Path,
    workspace: &str,
    window: (u64, u64),
) -> read::SessionReading {
    const NO_DATA: read::SessionReading = read::SessionReading {
        tokens: 0,
        context_pct: None,
    };
    let Some(harness) = resolve_harness(home) else {
        return NO_DATA;
    };
    let Some(projects_dir) = harness.session_logs_dir(home) else {
        return NO_DATA;
    };
    let logdir = projects_dir.join(harness.escape_workspace(workspace));
    let listing = list_jsonl(&logdir);
    let Some(file_name) = read::pick_log(&listing, window) else {
        return NO_DATA;
    };
    let tail = read_tail(&logdir.join(file_name), TAIL_BYTES);
    read::project_session(&tail, None)
}

/// The first registry harness that keeps session logs under `home`.
/// Matching a session to its *specific* harness, once more than one is
/// installed, is a documented future refinement.
fn resolve_harness(home: &Path) -> Option<Box<dyn Harness>> {
    willie_harness::registry()
        .into_iter()
        .find(|h| h.session_logs_dir(home).is_some())
}

/// Every `*.jsonl` file directly under `dir`, paired with its modified
/// time in whole seconds since the epoch. A missing or unreadable
/// directory is an empty listing, not a fault.
fn list_jsonl(dir: &Path) -> Vec<(String, u64)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| {
            entry.path().extension().and_then(|e| e.to_str()) == Some("jsonl")
        })
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            let mtime = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            Some((name, mtime))
        })
        .collect()
}

/// The last `max_bytes` of `path`, decoded as UTF-8 with lossy
/// replacement. Any I/O failure (the file vanished mid-read, a
/// permission fault) yields an empty tail, which `read::project_session`
/// already treats as "no usage record found".
fn read_tail(path: &Path, max_bytes: u64) -> String {
    let Ok(mut file) = fs::File::open(path) else {
        return String::new();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(max_bytes);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return String::new();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Whole seconds since the Unix epoch — plain `std`, no date-time
/// dependency. Also `fetched_at`'s format: a decimal epoch-seconds
/// string.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// --------------------------------------------------------------- helpers

fn json_response(
    value: &impl serde::Serialize,
) -> Result<PluginResponse, PluginError> {
    serde_json::to_value(value)
        .map(PluginResponse::json)
        .map_err(|e| {
            PluginError::Internal(format!("cannot encode response: {e}"))
        })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use serde_json::json;
    use willie_plugin_api::PluginEmission;

    use super::*;

    /// A unique, empty scratch directory standing in for a session's
    /// `home`, under the system temp root.
    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("willie-usage-plugin-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn noop_emit(_: PluginEmission) {}

    fn req(method: &str, params: Value) -> PluginRequest {
        PluginRequest {
            method: method.to_owned(),
            params,
        }
    }

    #[test]
    fn usage_is_a_global_plugin() {
        assert_eq!(UsagePlugin.manifest().scope, Scope::Global);
    }

    /// A JSONL planted at the exact path a registry harness resolves to
    /// (`session_logs_dir`/`escape_workspace`) is read and projected into
    /// the session's token count; the assistant record's own
    /// `message.model` resolves a real `context_pct` end to end, proving
    /// `read_session` no longer forces the window to `None`.
    #[test]
    fn snapshot_reads_tokens_and_context_pct_from_a_planted_session_log() {
        let home = scratch_dir("present");
        let harness = willie_harness::registry().remove(0);
        let workspace = "/home/willie/projects/x";
        let logdir = harness
            .session_logs_dir(&home)
            .unwrap()
            .join(harness.escape_workspace(workspace));
        fs::create_dir_all(&logdir).unwrap();
        fs::write(
            logdir.join("a.jsonl"),
            r#"{"message":{"model":"claude-sonnet-4-20250514","usage":{"input_tokens":50000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":5}}}"#,
        )
        .unwrap();

        let ctx = PluginCtx::new(&home, &noop_emit);
        let mut plugin = UsagePlugin;
        let session_id = SessionId::new();
        let project_id = ProjectId::new();
        let params = json!({
            "_home": home.to_string_lossy(),
            "_sessions": [{
                "id": session_id,
                "project_id": project_id,
                "workspace": workspace,
                "window": [0, null],
            }],
        });

        let resp = plugin.handle(&ctx, req("usage.snapshot", params)).unwrap();
        let snapshot: UsageSnapshot =
            serde_json::from_value(resp.into_value()).unwrap();

        assert_eq!(
            snapshot.sessions,
            vec![SessionUsage {
                id: session_id,
                tokens: 50_005,
                context_pct: Some(25),
            }]
        );
        assert_eq!(
            snapshot.projects,
            vec![ProjectUsage {
                id: project_id,
                tokens: 50_005,
            }]
        );

        let _ = fs::remove_dir_all(&home);
    }

    /// A session whose harness log directory was never created is still
    /// present in `sessions`, with zero tokens and no context percent —
    /// "shown with no usage data", not omitted.
    #[test]
    fn snapshot_a_missing_log_dir_is_present_with_zero_tokens() {
        let home = scratch_dir("absent");
        let ctx = PluginCtx::new(&home, &noop_emit);
        let mut plugin = UsagePlugin;
        let session_id = SessionId::new();
        let project_id = ProjectId::new();
        let params = json!({
            "_home": home.to_string_lossy(),
            "_sessions": [{
                "id": session_id,
                "project_id": project_id,
                "workspace": "/home/willie/projects/never-logged",
                "window": [0, null],
            }],
        });

        let resp = plugin.handle(&ctx, req("usage.snapshot", params)).unwrap();
        let snapshot: UsageSnapshot =
            serde_json::from_value(resp.into_value()).unwrap();

        assert_eq!(
            snapshot.sessions,
            vec![SessionUsage {
                id: session_id,
                tokens: 0,
                context_pct: None,
            }]
        );
        assert_eq!(
            snapshot.projects,
            vec![ProjectUsage {
                id: project_id,
                tokens: 0,
            }]
        );

        let _ = fs::remove_dir_all(&home);
    }

    /// A bad/missing params shape never fails the call: it simply
    /// yields no sessions.
    #[test]
    fn snapshot_with_no_sessions_is_an_empty_snapshot() {
        let home = scratch_dir("empty-params");
        let ctx = PluginCtx::new(&home, &noop_emit);
        let mut plugin = UsagePlugin;

        let resp = plugin
            .handle(&ctx, req("usage.snapshot", Value::Null))
            .unwrap();
        let snapshot: UsageSnapshot =
            serde_json::from_value(resp.into_value()).unwrap();

        assert!(snapshot.sessions.is_empty());
        assert!(snapshot.projects.is_empty());
        assert!(snapshot.providers.is_empty());

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn an_unknown_method_is_a_bad_request() {
        let home = scratch_dir("bad-method");
        let ctx = PluginCtx::new(&home, &noop_emit);
        let err = UsagePlugin
            .handle(&ctx, req("usage.frobnicate", Value::Null))
            .unwrap_err();
        assert_eq!(err.code(), "plugin_bad_request");
        let _ = fs::remove_dir_all(&home);
    }

    /// `SessionStarted`/`SessionExited` each emit the no-id
    /// `usage.updated` signal; the client re-pulls a snapshot on it.
    #[test]
    fn on_event_emits_usage_updated_for_started_and_exited() {
        let home = scratch_dir("on-event");
        let captured: RefCell<Vec<PluginEmission>> = RefCell::new(Vec::new());
        let sink =
            |emission: PluginEmission| captured.borrow_mut().push(emission);
        let ctx = PluginCtx::new(&home, &sink);
        let mut plugin = UsagePlugin;

        plugin.on_event(
            &ctx,
            &CoreEvent::SessionExited {
                session_id: SessionId::new(),
            },
        );
        plugin.on_event(
            &ctx,
            &CoreEvent::SessionStarted {
                session_id: SessionId::new(),
                project_id: ProjectId::new(),
            },
        );

        let emitted = captured.borrow();
        assert_eq!(emitted.len(), 2);
        assert!(emitted.iter().all(|e| e.kind == "usage.updated"));
        assert!(emitted.iter().all(|e| e.payload == json!({})));

        let _ = fs::remove_dir_all(&home);
    }

    /// `ProjectRegistered` and `Tick` are not usage-relevant events: no
    /// emission.
    #[test]
    fn on_event_ignores_project_registered_and_tick() {
        let home = scratch_dir("on-event-ignored");
        let captured: RefCell<Vec<PluginEmission>> = RefCell::new(Vec::new());
        let sink =
            |emission: PluginEmission| captured.borrow_mut().push(emission);
        let ctx = PluginCtx::new(&home, &sink);
        let mut plugin = UsagePlugin;

        plugin.on_event(
            &ctx,
            &CoreEvent::ProjectRegistered {
                project_id: ProjectId::new(),
            },
        );
        plugin.on_event(&ctx, &CoreEvent::Tick);

        assert!(captured.borrow().is_empty());

        let _ = fs::remove_dir_all(&home);
    }
}
