//! Contract between the daemon and its plugins.
//!
//! Plugins are compiled into the daemon. They never run inside a session
//! sandbox, never touch daemon internals, and a failing plugin degrades
//! only itself. The daemon exposes what a plugin needs through a context
//! object that later revisions of this crate will grow.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use willie_core::id::{ProjectId, SessionId};

/// Where a plugin can be enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// One instance for the whole installation.
    Global,
    /// Enabled and configured per project.
    PerProject,
}

/// Static description of a plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginManifest {
    /// Stable identifier used in RPC namespaces, e.g. `usage`.
    pub id: &'static str,
    /// Human-readable name.
    pub name: &'static str,
    /// Where the plugin can be enabled.
    pub scope: Scope,
}

/// Failure a plugin call returns to the host.
///
/// Every variant carries a stable, `snake_case` code (see
/// [`code`](Self::code)) that the UI and the CLI can act on, and a
/// [`remediation`](Self::remediation) hint for the person hitting it. A
/// plugin whose failure deserves a code of its own to key behaviour on —
/// `profile_exists`, say — builds it with [`PluginError::coded`] instead
/// of reaching for [`PluginError::Internal`].
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    /// The plugin failed for a reason it has no code of its own for.
    /// Point of last resort, not a substitute for a coded variant a
    /// caller can act on.
    #[error("internal plugin error: {0}")]
    Internal(String),
    /// The caller's request was malformed: an unknown method, or missing
    /// or ill-typed params.
    #[error("bad request: {0}")]
    BadRequest(String),
    /// A plugin-specific failure, built through [`PluginError::coded`].
    #[error("{message}")]
    Coded {
        /// Stable `snake_case` code, e.g. `profile_exists`.
        code: String,
        /// One sentence describing what went wrong.
        message: String,
        /// What the caller can do about it.
        remediation: String,
    },
}

impl PluginError {
    /// Builds a plugin-specific error carrying its own stable code and
    /// remediation, for the failures a plugin can name precisely.
    #[must_use]
    pub fn coded(
        code: impl Into<String>,
        message: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        Self::Coded {
            code: code.into(),
            message: message.into(),
            remediation: remediation.into(),
        }
    }

    /// Stable machine-readable code.
    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::Internal(_) => "plugin_internal",
            Self::BadRequest(_) => "plugin_bad_request",
            Self::Coded { code, .. } => code,
        }
    }

    /// What the caller can do about it.
    #[must_use]
    pub fn remediation(&self) -> &str {
        match self {
            Self::Internal(_) => "retry; if it repeats, check the daemon log",
            Self::BadRequest(_) => {
                "check the request's method and parameters and retry"
            }
            Self::Coded { remediation, .. } => remediation,
        }
    }
}

/// One `<id>.<method>` call routed to a plugin.
///
/// Mirrors the shape of a JSON-RPC call (`willie_proto::rpc::Request`)
/// closely enough that it round-trips through JSON, for a plugin that
/// logs or replays what it was asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginRequest {
    /// Dot-qualified method name the caller asked for, e.g.
    /// `profile.list`. The host has already resolved the leading `<id>`
    /// to find this plugin; the field keeps the full name so a plugin
    /// hosting more than one namespace can still branch on it.
    pub method: String,
    /// The call's parameters, opaque to the host.
    #[serde(default)]
    pub params: serde_json::Value,
}

/// Reply to a [`PluginRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginResponse {
    /// The only shape the first cut needs: a JSON value the host forwards
    /// as the RPC result.
    Json(serde_json::Value),
}

impl PluginResponse {
    /// Wraps a JSON value as a response.
    #[must_use]
    pub fn json(value: serde_json::Value) -> Self {
        Self::Json(value)
    }

    /// Unwraps the response back into its JSON value.
    #[must_use]
    pub fn into_value(self) -> serde_json::Value {
        match self {
            Self::Json(value) => value,
        }
    }
}

/// A daemon-wide happening a plugin may want to react to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreEvent {
    /// A session started running in a project.
    SessionStarted {
        session_id: SessionId,
        project_id: ProjectId,
    },
    /// A session's process exited.
    SessionExited { session_id: SessionId },
    /// A project was registered with the daemon.
    ProjectRegistered { project_id: ProjectId },
    /// A periodic tick, for plugins with no event of their own to key
    /// polling on.
    Tick,
}

/// One event a plugin emits; the daemon forwards it as a
/// `plugin.emitted` notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEmission {
    /// The plugin-defined kind, e.g. `sync_progress`.
    pub kind: String,
    /// The event's payload, opaque to the host.
    pub payload: serde_json::Value,
}

/// What the daemon gives a plugin for one call into it.
///
/// The first cut carries what the profiles plugin needs, no more: a
/// private store directory and a way to emit an event. The HTTP client
/// and the scheduler are the next fields this grows — usage's (F5) — and
/// are deliberately not built here; a plugin that needs them waits for
/// that slice.
pub struct PluginCtx<'a> {
    /// `/var/lib/willie/plugins/<id>/`, created on first use. This
    /// plugin's own private tree; nothing else reads it.
    pub store_dir: &'a Path,
    /// Emits a plugin event the daemon forwards as a notification.
    emit: &'a dyn Fn(PluginEmission),
}

impl<'a> PluginCtx<'a> {
    /// Builds a context rooted at `store_dir`, forwarding every `emit`
    /// call to `emit`.
    #[must_use]
    pub fn new(store_dir: &'a Path, emit: &'a dyn Fn(PluginEmission)) -> Self {
        Self { store_dir, emit }
    }

    /// This plugin's private store directory.
    #[must_use]
    pub fn store_dir(&self) -> &Path {
        self.store_dir
    }

    /// Emits a plugin event the daemon forwards as a notification.
    pub fn emit(&self, kind: &str, payload: serde_json::Value) {
        (self.emit)(PluginEmission {
            kind: kind.to_owned(),
            payload,
        });
    }
}

impl fmt::Debug for PluginCtx<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PluginCtx")
            .field("store_dir", &self.store_dir)
            .finish_non_exhaustive()
    }
}

/// A capability hosted by the daemon.
///
/// Plugins are compiled into the daemon (§4.1): the registry is a plain
/// `Vec<Box<dyn Plugin>>`, not a dynamic-loading mechanism. Every method
/// but [`manifest`](Self::manifest) and [`handle`](Self::handle) defaults
/// to doing nothing, so a plugin with no lifecycle or event needs (the
/// usage stub, for now) implements only those two.
///
/// [`PluginCtx`] carries what this first cut needs: a private store
/// directory and `emit`. The HTTP client and the scheduler are the next
/// fields it grows — usage's (F5) — and are deliberately NOT built here;
/// a plugin that needs them waits for that slice.
pub trait Plugin: fmt::Debug {
    /// Static description.
    fn manifest(&self) -> PluginManifest;

    /// Called once when the plugin is enabled in `scope` (a per-project
    /// plugin, once per project it is enabled in). The default does
    /// nothing.
    fn on_enable(
        &mut self,
        _ctx: &PluginCtx<'_>,
        _scope: Scope,
    ) -> Result<(), PluginError> {
        Ok(())
    }

    /// Called once when the plugin is disabled in `scope`. The default
    /// does nothing.
    fn on_disable(
        &mut self,
        _ctx: &PluginCtx<'_>,
        _scope: Scope,
    ) -> Result<(), PluginError> {
        Ok(())
    }

    /// Handles one `<id>.<method>` call routed to this plugin. Required:
    /// even a plugin with nothing to answer still needs to say so.
    fn handle(
        &mut self,
        ctx: &PluginCtx<'_>,
        req: PluginRequest,
    ) -> Result<PluginResponse, PluginError>;

    /// Notified of a daemon-wide event. The default ignores it.
    fn on_event(&mut self, _ctx: &PluginCtx<'_>, _ev: &CoreEvent) {}
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[derive(Debug)]
    struct Probe;

    impl Plugin for Probe {
        fn manifest(&self) -> PluginManifest {
            PluginManifest {
                id: "probe",
                name: "Probe",
                scope: Scope::Global,
            }
        }

        // Only `handle` is implemented: `on_enable`, `on_disable` and
        // `on_event` are exercised through their default bodies below.
        fn handle(
            &mut self,
            _ctx: &PluginCtx<'_>,
            req: PluginRequest,
        ) -> Result<PluginResponse, PluginError> {
            Ok(PluginResponse::json(req.params))
        }
    }

    #[test]
    fn manifest_is_reachable_through_the_trait_object() {
        let plugin: Box<dyn Plugin> = Box::new(Probe);
        assert_eq!(plugin.manifest().id, "probe");
    }

    // --- grown contract: PluginError, PluginCtx, handle, lifecycle ---

    #[test]
    fn handle_echoes_the_request_params() {
        let dir = Path::new("/var/lib/willie/plugins/probe");
        let ctx = PluginCtx::new(dir, &|_| {});
        let mut plugin = Probe;
        let req = PluginRequest {
            method: "probe.echo".into(),
            params: serde_json::json!({"x": 1}),
        };

        let resp = plugin.handle(&ctx, req).unwrap();

        assert_eq!(resp.into_value(), serde_json::json!({"x": 1}));
    }

    #[test]
    fn default_on_enable_and_on_event_do_nothing_and_compile() {
        let dir = Path::new("/var/lib/willie/plugins/probe");
        let ctx = PluginCtx::new(dir, &|_| {});
        let mut plugin = Probe;

        assert!(plugin.on_enable(&ctx, Scope::Global).is_ok());
        assert!(plugin.on_disable(&ctx, Scope::Global).is_ok());
        plugin.on_event(&ctx, &CoreEvent::Tick);
    }

    #[test]
    fn ctx_yields_the_store_dir_it_was_built_with() {
        let dir = Path::new("/var/lib/willie/plugins/probe");
        let ctx = PluginCtx::new(dir, &|_| {});
        assert_eq!(ctx.store_dir(), dir);
    }

    #[test]
    fn ctx_emit_forwards_the_kind_and_payload() {
        let dir = Path::new("/var/lib/willie/plugins/probe");
        let captured: RefCell<Vec<PluginEmission>> = RefCell::new(Vec::new());
        let sink =
            |emission: PluginEmission| captured.borrow_mut().push(emission);
        let ctx = PluginCtx::new(dir, &sink);

        ctx.emit("tick", serde_json::json!({"n": 1}));

        let emitted = captured.borrow();
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].kind, "tick");
        assert_eq!(emitted[0].payload, serde_json::json!({"n": 1}));
    }

    #[test]
    fn a_request_round_trips_through_json() {
        let req = PluginRequest {
            method: "profile.list".into(),
            params: serde_json::json!({"project_id": "proj_1"}),
        };

        let json = serde_json::to_string(&req).unwrap();
        let back: PluginRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(back, req);
    }

    #[test]
    fn a_request_with_no_params_field_defaults_to_null() {
        let req: PluginRequest =
            serde_json::from_str(r#"{"method":"profile.list"}"#).unwrap();
        assert_eq!(req.params, serde_json::Value::Null);
    }

    #[test]
    fn an_emission_round_trips_through_json() {
        let emission = PluginEmission {
            kind: "sync_progress".into(),
            payload: serde_json::json!({"percent": 42}),
        };

        let json = serde_json::to_string(&emission).unwrap();
        let back: PluginEmission = serde_json::from_str(&json).unwrap();

        assert_eq!(back, emission);
    }

    #[test]
    fn response_json_round_trips_into_value() {
        let value = serde_json::json!({"ok": true});
        let resp = PluginResponse::json(value.clone());
        assert_eq!(resp.into_value(), value);
    }

    #[test]
    fn internal_and_bad_request_have_stable_codes() {
        assert_eq!(PluginError::Internal("x".into()).code(), "plugin_internal");
        assert_eq!(
            PluginError::BadRequest("x".into()).code(),
            "plugin_bad_request"
        );
    }

    #[test]
    fn coded_carries_its_own_code_and_remediation() {
        let err = PluginError::coded(
            "profile_exists",
            "a profile named x already exists",
            "pick a different name",
        );
        assert_eq!(err.code(), "profile_exists");
        assert_eq!(err.remediation(), "pick a different name");
    }

    #[test]
    fn every_error_has_a_nonempty_remediation() {
        let errors = [
            PluginError::Internal("x".into()),
            PluginError::BadRequest("x".into()),
            PluginError::coded("c", "m", "r"),
        ];
        for err in errors {
            assert!(!err.remediation().is_empty());
        }
    }

    #[test]
    fn core_event_variants_are_constructible() {
        let session_id = SessionId::new();
        let project_id = ProjectId::new();

        let started = CoreEvent::SessionStarted {
            session_id,
            project_id,
        };
        let exited = CoreEvent::SessionExited { session_id };
        let registered = CoreEvent::ProjectRegistered { project_id };
        let tick = CoreEvent::Tick;

        assert!(matches!(started, CoreEvent::SessionStarted { .. }));
        assert!(matches!(exited, CoreEvent::SessionExited { .. }));
        assert!(matches!(registered, CoreEvent::ProjectRegistered { .. }));
        assert!(matches!(tick, CoreEvent::Tick));
    }
}
