//! The plugin host: the daemon's registry of compiled-in plugins, which
//! of them are enabled (and, for a per-project plugin, in which projects),
//! and the routing of `plugin.*`/`<id>.*` calls to them.
//!
//! Plugins run inside the daemon, outside every session sandbox, and each
//! degrades only itself. A plugin is marked `degraded` only when a call
//! into it panics or returns an internal fault; a coded refusal (a
//! legitimate "no", e.g. `profile_exists`) is not a malfunction and leaves
//! it healthy. The panic is caught at this boundary so one plugin's crash
//! never takes the daemon down. Enablement is a file under
//! `<state_dir>/plugins/enabled.toml`; a missing or unreadable file reads
//! as "nothing enabled" (a plugin being off is the safe default).

use std::{
    collections::{BTreeMap, BTreeSet},
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use willie_core::id::ProjectId;
use willie_plugin_api::{
    CoreEvent, Plugin, PluginCtx, PluginEmission, PluginError, PluginRequest,
    Scope as ApiScope,
};
use willie_proto::plugin::{
    EnableParams, Enablement, PluginStatus, Scope as WireScope,
};

use crate::projects::OpError;

/// The compiled-in plugins, in a stable order. The registry is a plain
/// `Vec` (§4.1): plugins are not dynamically loaded.
fn registry() -> Vec<Box<dyn Plugin + Send>> {
    vec![
        Box::new(willie_plugin_profiles::plugin()),
        Box::new(willie_plugin_usage::plugin()),
    ]
}

/// `<state_dir>/plugins/`.
fn plugins_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("plugins")
}

/// `<state_dir>/plugins/enabled.toml`.
fn enabled_path(state_dir: &Path) -> PathBuf {
    plugins_dir(state_dir).join("enabled.toml")
}

/// A plugin's private store, `<state_dir>/plugins/<id>/`.
fn store_dir_for(state_dir: &Path, id: &str) -> PathBuf {
    plugins_dir(state_dir).join(id)
}

/// One plugin's on-disk enablement row. A `Global` plugin uses `global`;
/// a `PerProject` plugin uses `projects`. Kept as its own file shape (not
/// the wire `Enablement`) so `enabled.toml` reads the way the design shows
/// it: `[<id>]` with `global = <bool>` or `projects = [<ids>]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct EnabledEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    global: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    projects: Vec<ProjectId>,
}

/// The whole `enabled.toml`: one `[<id>]` table per enabled plugin.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
struct EnabledState {
    plugins: BTreeMap<String, EnabledEntry>,
}

impl EnabledState {
    /// Loads the record, or an empty one on any failure — a missing or
    /// unreadable file means "nothing enabled", and the daemon still runs.
    fn load(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        match toml::from_str(&text) {
            Ok(plugins) => Self { plugins },
            Err(e) => {
                eprintln!(
                    "willied: cannot parse {}: {e}; no plugins enabled",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Rewrites the record, logging rather than failing: an enablement that
    /// cannot be persisted still took effect in memory for this run.
    fn save(&self, path: &Path) {
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            eprintln!(
                "willied: cannot create {}: {e}; enablement not persisted",
                parent.display()
            );
            return;
        }
        match toml::to_string(&self.plugins) {
            Ok(text) => {
                if let Err(e) = std::fs::write(path, text) {
                    eprintln!(
                        "willied: cannot write {}: {e}; enablement not \
                         persisted",
                        path.display()
                    );
                }
            }
            Err(e) => eprintln!("willied: cannot encode enabled.toml: {e}"),
        }
    }
}

/// The host: the plugins, their enablement, the set of ids currently
/// degraded, and the state directory their storage roots under.
pub struct PluginHost {
    plugins: Vec<Box<dyn Plugin + Send>>,
    enabled: EnabledState,
    degraded: BTreeSet<String>,
    state_dir: PathBuf,
}

impl std::fmt::Debug for PluginHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginHost")
            .field("plugins", &self.plugins.len())
            .field("degraded", &self.degraded)
            .finish_non_exhaustive()
    }
}

impl PluginHost {
    /// Builds the host over the compiled-in registry, loading enablement
    /// from `<state_dir>/plugins/enabled.toml`.
    #[must_use]
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        let state_dir = state_dir.into();
        Self::with_registry(state_dir, registry())
    }

    fn with_registry(
        state_dir: PathBuf,
        plugins: Vec<Box<dyn Plugin + Send>>,
    ) -> Self {
        let enabled = EnabledState::load(&enabled_path(&state_dir));
        Self {
            plugins,
            enabled,
            degraded: BTreeSet::new(),
            state_dir,
        }
    }

    /// Every plugin's manifest merged with its enablement and degraded flag.
    #[must_use]
    pub fn list(&self) -> Vec<PluginStatus> {
        (0..self.plugins.len())
            .map(|idx| self.status_of(idx))
            .collect()
    }

    /// Enables `params.id` in the scope its `project_id` implies, refusing a
    /// scope its manifest forbids. Runs `on_enable` inside the panic
    /// boundary, persists, and returns the new status.
    pub fn enable(
        &mut self,
        params: EnableParams,
    ) -> Result<PluginStatus, OpError> {
        self.set_enabled(params, true)
    }

    /// Disables `params.id` in the scope its `project_id` implies.
    pub fn disable(
        &mut self,
        params: EnableParams,
    ) -> Result<PluginStatus, OpError> {
        self.set_enabled(params, false)
    }

    /// The shared body of `enable`/`disable`: validate the scope, run the
    /// lifecycle hook inside the panic boundary, record the change and
    /// persist. A hook that fails does not persist the change; a panic or an
    /// internal fault marks the plugin degraded, while a coded refusal
    /// passes through without degrading it.
    fn set_enabled(
        &mut self,
        params: EnableParams,
        on: bool,
    ) -> Result<PluginStatus, OpError> {
        let EnableParams { id, project_id } = params;
        let Some(idx) = self.index_of(&id) else {
            return Err(not_found(&id));
        };
        let scope = self.plugins[idx].manifest().scope;

        match (scope, project_id) {
            (ApiScope::Global, None) | (ApiScope::PerProject, Some(_)) => {}
            (ApiScope::Global, Some(_)) => {
                return Err(op(
                    "plugin_scope_mismatch",
                    format!(
                        "the `{id}` plugin is global and cannot be enabled \
                         for a single project"
                    ),
                    "enable it globally, with no project id",
                ));
            }
            (ApiScope::PerProject, None) => {
                return Err(op(
                    "plugin_scope_mismatch",
                    format!(
                        "the `{id}` plugin is per-project and cannot be \
                         enabled globally"
                    ),
                    "enable it for a project by passing its project id",
                ));
            }
        }

        let store_dir = store_dir_for(&self.state_dir, &id);
        let _ = std::fs::create_dir_all(&store_dir);
        let emit = |_emission: PluginEmission| {};
        let ctx = PluginCtx::new(&store_dir, &emit);
        let fired = if on {
            guarded(|| self.plugins[idx].on_enable(&ctx, scope))
        } else {
            guarded(|| self.plugins[idx].on_disable(&ctx, scope))
        };

        match fired {
            Err(()) => {
                self.degraded.insert(id.clone());
                return Err(panicked(&id));
            }
            Ok(Err(e)) => {
                if is_fault(&e) {
                    self.degraded.insert(id.clone());
                }
                return Err(plugin_error_to_op(e));
            }
            Ok(Ok(())) => {
                self.degraded.remove(&id);
            }
        }

        let entry = self.enabled.plugins.entry(id.clone()).or_default();
        match project_id {
            None => entry.global = Some(on),
            Some(project) if on => {
                if !entry.projects.contains(&project) {
                    entry.projects.push(project);
                }
            }
            Some(project) => entry.projects.retain(|p| *p != project),
        }
        self.enabled.save(&enabled_path(&self.state_dir));

        Ok(self.status_of(idx))
    }

    /// Routes `<id>.<method>` to its plugin. An unknown id is
    /// `plugin_not_found`, a disabled plugin `plugin_disabled`. A panic is
    /// caught and answered `plugin_panicked`, marking the plugin degraded; a
    /// returned internal fault also marks it degraded, while a coded refusal
    /// passes through as its `OpError` and leaves the plugin healthy.
    pub fn handle(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<Value, OpError> {
        let id = method.split_once('.').map_or(method, |(id, _)| id);
        let Some(idx) = self.index_of(id) else {
            return Err(not_found(id));
        };
        if !self.is_enabled(idx) {
            return Err(disabled(id));
        }

        let store_dir = store_dir_for(&self.state_dir, id);
        let _ = std::fs::create_dir_all(&store_dir);
        let emit = |_emission: PluginEmission| {};
        let ctx = PluginCtx::new(&store_dir, &emit);
        let req = PluginRequest {
            method: method.to_owned(),
            params,
        };
        let called = guarded(|| self.plugins[idx].handle(&ctx, req));

        match called {
            Err(()) => {
                self.degraded.insert(id.to_owned());
                Err(panicked(id))
            }
            Ok(Err(e)) => {
                if is_fault(&e) {
                    self.degraded.insert(id.to_owned());
                }
                Err(plugin_error_to_op(e))
            }
            Ok(Ok(response)) => {
                self.degraded.remove(id);
                Ok(response.into_value())
            }
        }
    }

    /// Fans a `CoreEvent` to every enabled plugin inside the panic boundary.
    /// A plugin that panics is marked degraded; the daemon continues.
    pub fn on_event(&mut self, ev: CoreEvent) {
        for idx in 0..self.plugins.len() {
            if !self.is_enabled(idx) {
                continue;
            }
            let id = self.plugins[idx].manifest().id;
            let store_dir = store_dir_for(&self.state_dir, id);
            let _ = std::fs::create_dir_all(&store_dir);
            let emit = |_emission: PluginEmission| {};
            let ctx = PluginCtx::new(&store_dir, &emit);
            let fired = guarded(|| self.plugins[idx].on_event(&ctx, &ev));
            if fired.is_err() {
                self.degraded.insert(id.to_owned());
            }
        }
    }

    /// The index of the plugin with this id.
    fn index_of(&self, id: &str) -> Option<usize> {
        self.plugins.iter().position(|p| p.manifest().id == id)
    }

    /// Whether the plugin at `idx` is enabled: a global plugin's flag, or a
    /// per-project plugin being enabled in at least one project.
    fn is_enabled(&self, idx: usize) -> bool {
        let manifest = self.plugins[idx].manifest();
        let entry = self.enabled.plugins.get(manifest.id);
        match manifest.scope {
            ApiScope::Global => entry.and_then(|e| e.global).unwrap_or(false),
            ApiScope::PerProject => {
                entry.map(|e| !e.projects.is_empty()).unwrap_or(false)
            }
        }
    }

    /// The status of the plugin at `idx`.
    fn status_of(&self, idx: usize) -> PluginStatus {
        let manifest = self.plugins[idx].manifest();
        let entry = self.enabled.plugins.get(manifest.id);
        let enabled = match manifest.scope {
            ApiScope::Global => Enablement::Global(
                entry.and_then(|e| e.global).unwrap_or(false),
            ),
            ApiScope::PerProject => Enablement::PerProject(
                entry.map(|e| e.projects.clone()).unwrap_or_default(),
            ),
        };
        PluginStatus {
            id: manifest.id.to_owned(),
            name: manifest.name.to_owned(),
            scope: wire_scope(manifest.scope),
            enabled,
            degraded: self.degraded.contains(manifest.id),
        }
    }
}

/// Runs `f` inside a panic boundary. `Err(())` means it panicked; the
/// caller marks the plugin degraded. `AssertUnwindSafe` is required because
/// the closure calls through a `&mut dyn Plugin`.
fn guarded<T>(f: impl FnOnce() -> T) -> Result<T, ()> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|_| ())
}

/// Maps the plugin API's `Scope` onto the wire copy the proto owns.
fn wire_scope(scope: ApiScope) -> WireScope {
    match scope {
        ApiScope::Global => WireScope::Global,
        ApiScope::PerProject => WireScope::PerProject,
    }
}

/// Maps the wire `Scope` back onto the plugin API's.
#[cfg_attr(not(test), allow(dead_code))]
fn api_scope(scope: WireScope) -> ApiScope {
    match scope {
        WireScope::Global => ApiScope::Global,
        WireScope::PerProject => ApiScope::PerProject,
    }
}

fn op(code: &str, message: String, remediation: &str) -> OpError {
    OpError {
        code: code.to_owned(),
        message,
        remediation: remediation.to_owned(),
    }
}

fn not_found(id: &str) -> OpError {
    op(
        "plugin_not_found",
        format!("no plugin with id `{id}`"),
        "check plugin.list for the available plugin ids",
    )
}

fn disabled(id: &str) -> OpError {
    op(
        "plugin_disabled",
        format!("the `{id}` plugin is disabled"),
        "enable it with plugin.enable before calling its methods",
    )
}

fn panicked(id: &str) -> OpError {
    op(
        "plugin_panicked",
        format!("the `{id}` plugin panicked and was marked degraded"),
        "check the daemon log; the plugin stays degraded until it is \
         re-enabled",
    )
}

/// Whether a plugin error is a genuine fault the plugin should be marked
/// degraded for. `Internal` is a malfunction; a `Coded` refusal (e.g.
/// `profile_exists`) or a `BadRequest` is a legitimate "no" answer that
/// leaves the plugin healthy, so it is not a fault.
fn is_fault(e: &PluginError) -> bool {
    matches!(e, PluginError::Internal(_))
}

/// Maps a `PluginError` onto the host's `OpError`, preserving its code and
/// remediation so the wire reply carries what the plugin chose.
fn plugin_error_to_op(e: PluginError) -> OpError {
    OpError {
        code: e.code().to_owned(),
        message: e.to_string(),
        remediation: e.remediation().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use willie_plugin_api::{
        PluginManifest, PluginResponse, Scope as ApiScope,
    };

    use super::*;

    /// A unique, empty state directory under the system temp root. ULID
    /// names keep parallel test runs from colliding; the host creates the
    /// `plugins/` tree under it on first use.
    fn tmp_state_dir() -> PathBuf {
        let dir = std::env::temp_dir()
            .join("willie-plugins-test")
            .join(ProjectId::new().to_string());
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn global(id: &'static str) -> EnableParams {
        EnableParams {
            id: id.to_owned(),
            project_id: None,
        }
    }

    fn per_project(id: &'static str, project: ProjectId) -> EnableParams {
        EnableParams {
            id: id.to_owned(),
            project_id: Some(project),
        }
    }

    /// A global plugin whose `handle` returns an internal fault — a genuine
    /// malfunction, for the degraded path.
    #[derive(Debug)]
    struct Faulting;

    impl Plugin for Faulting {
        fn manifest(&self) -> PluginManifest {
            PluginManifest {
                id: "faulting",
                name: "Faulting",
                scope: ApiScope::Global,
            }
        }

        fn handle(
            &mut self,
            _ctx: &PluginCtx<'_>,
            _req: PluginRequest,
        ) -> Result<PluginResponse, PluginError> {
            Err(PluginError::Internal("this plugin malfunctioned".into()))
        }
    }

    /// A global plugin whose `handle` returns a coded refusal — a legitimate
    /// "no", not a malfunction, so it must not degrade the plugin.
    #[derive(Debug)]
    struct Refusing;

    impl Plugin for Refusing {
        fn manifest(&self) -> PluginManifest {
            PluginManifest {
                id: "refusing",
                name: "Refusing",
                scope: ApiScope::Global,
            }
        }

        fn handle(
            &mut self,
            _ctx: &PluginCtx<'_>,
            _req: PluginRequest,
        ) -> Result<PluginResponse, PluginError> {
            Err(PluginError::coded(
                "refusing_no",
                "this plugin says no",
                "nothing to fix; it is a refusal, not a fault",
            ))
        }
    }

    /// A global plugin whose `handle` panics, for the panic boundary.
    #[derive(Debug)]
    struct HandlePanics;

    impl Plugin for HandlePanics {
        fn manifest(&self) -> PluginManifest {
            PluginManifest {
                id: "panic_handle",
                name: "Panic on handle",
                scope: ApiScope::Global,
            }
        }

        fn handle(
            &mut self,
            _ctx: &PluginCtx<'_>,
            _req: PluginRequest,
        ) -> Result<PluginResponse, PluginError> {
            panic!("handle panicked on purpose");
        }
    }

    /// A global plugin whose `on_event` panics; `handle` is never reached.
    #[derive(Debug)]
    struct EventPanics;

    impl Plugin for EventPanics {
        fn manifest(&self) -> PluginManifest {
            PluginManifest {
                id: "panic_event",
                name: "Panic on event",
                scope: ApiScope::Global,
            }
        }

        fn handle(
            &mut self,
            _ctx: &PluginCtx<'_>,
            _req: PluginRequest,
        ) -> Result<PluginResponse, PluginError> {
            Ok(PluginResponse::json(Value::Null))
        }

        fn on_event(&mut self, _ctx: &PluginCtx<'_>, _ev: &CoreEvent) {
            panic!("on_event panicked on purpose");
        }
    }

    #[test]
    fn the_registry_lists_profiles_and_usage() {
        let host = PluginHost::new(tmp_state_dir());
        let ids: Vec<String> = host.list().into_iter().map(|s| s.id).collect();
        assert_eq!(ids, vec!["profile".to_owned(), "usage".to_owned()]);
    }

    #[test]
    fn a_fresh_host_reports_everything_disabled() {
        let host = PluginHost::new(tmp_state_dir());
        for status in host.list() {
            match status.enabled {
                Enablement::Global(on) => assert!(!on, "{}", status.id),
                Enablement::PerProject(projects) => {
                    assert!(projects.is_empty(), "{}", status.id)
                }
            }
            assert!(!status.degraded, "{}", status.id);
        }
    }

    #[test]
    fn enabling_a_per_project_plugin_persists_and_round_trips() {
        let dir = tmp_state_dir();
        let project = ProjectId::new();

        let status = {
            let mut host = PluginHost::new(&dir);
            host.enable(per_project("profile", project)).unwrap()
        };
        assert_eq!(status.id, "profile");
        assert_eq!(status.enabled, Enablement::PerProject(vec![project]));

        // The record is on disk and a fresh host loads it.
        assert!(enabled_path(&dir).exists());
        let reloaded = PluginHost::new(&dir);
        let profiles = reloaded
            .list()
            .into_iter()
            .find(|s| s.id == "profile")
            .unwrap();
        assert_eq!(profiles.enabled, Enablement::PerProject(vec![project]));
    }

    #[test]
    fn disabling_removes_the_project_and_round_trips() {
        let dir = tmp_state_dir();
        let project = ProjectId::new();
        let mut host = PluginHost::new(&dir);
        host.enable(per_project("profile", project)).unwrap();

        let status = host.disable(per_project("profile", project)).unwrap();
        assert_eq!(status.enabled, Enablement::PerProject(vec![]));

        let reloaded = PluginHost::new(&dir);
        let profiles = reloaded
            .list()
            .into_iter()
            .find(|s| s.id == "profile")
            .unwrap();
        assert_eq!(profiles.enabled, Enablement::PerProject(vec![]));
    }

    #[test]
    fn enabling_a_global_plugin_persists_and_round_trips() {
        let dir = tmp_state_dir();
        let mut host = PluginHost::new(&dir);

        let status = host.enable(global("usage")).unwrap();
        assert_eq!(status.enabled, Enablement::Global(true));

        let reloaded = PluginHost::new(&dir);
        let usage = reloaded
            .list()
            .into_iter()
            .find(|s| s.id == "usage")
            .unwrap();
        assert_eq!(usage.enabled, Enablement::Global(true));
    }

    #[test]
    fn a_global_scope_on_a_per_project_plugin_is_a_scope_mismatch() {
        let mut host = PluginHost::new(tmp_state_dir());
        let err = host.enable(global("profile")).unwrap_err();
        assert_eq!(err.code, "plugin_scope_mismatch");
    }

    #[test]
    fn a_per_project_scope_on_a_global_plugin_is_a_scope_mismatch() {
        let mut host = PluginHost::new(tmp_state_dir());
        let err = host
            .enable(per_project("usage", ProjectId::new()))
            .unwrap_err();
        assert_eq!(err.code, "plugin_scope_mismatch");
    }

    #[test]
    fn enabling_an_unknown_id_is_plugin_not_found() {
        let mut host = PluginHost::new(tmp_state_dir());
        let err = host.enable(global("nope")).unwrap_err();
        assert_eq!(err.code, "plugin_not_found");
    }

    #[test]
    fn handling_an_unknown_id_is_plugin_not_found() {
        let mut host = PluginHost::new(tmp_state_dir());
        let err = host.handle("nope.list", Value::Null).unwrap_err();
        assert_eq!(err.code, "plugin_not_found");
    }

    #[test]
    fn handling_a_disabled_plugin_is_plugin_disabled() {
        let mut host = PluginHost::new(tmp_state_dir());
        let err = host.handle("usage.summary", Value::Null).unwrap_err();
        assert_eq!(err.code, "plugin_disabled");
    }

    #[test]
    fn handle_routes_profile_calls_to_the_profiles_plugin() {
        let mut host = PluginHost::new(tmp_state_dir());
        host.enable(per_project("profile", ProjectId::new()))
            .unwrap();

        // The profiles plugin answers `profile.list` with an empty array
        // when nothing has been created yet — enough to prove the call
        // reached its own `handle`, not a host-level stub.
        let result = host.handle("profile.list", Value::Null).unwrap();
        assert_eq!(result, Value::Array(vec![]));
    }

    #[test]
    fn a_fault_from_handle_marks_the_plugin_degraded_and_the_host_answers() {
        let dir = tmp_state_dir();
        let mut host = PluginHost::with_registry(dir, vec![Box::new(Faulting)]);
        host.enable(global("faulting")).unwrap();

        let err = host.handle("faulting.x", Value::Null).unwrap_err();
        assert_eq!(err.code, "plugin_internal");

        // The host still answers, and the plugin is degraded in its status.
        let status = host
            .list()
            .into_iter()
            .find(|s| s.id == "faulting")
            .unwrap();
        assert!(status.degraded);
    }

    #[test]
    fn a_coded_refusal_from_handle_does_not_mark_the_plugin_degraded() {
        let dir = tmp_state_dir();
        let mut host = PluginHost::with_registry(dir, vec![Box::new(Refusing)]);
        host.enable(global("refusing")).unwrap();

        let err = host.handle("refusing.x", Value::Null).unwrap_err();
        assert_eq!(err.code, "refusing_no");

        // A refusal is a legitimate "no", not a malfunction: the plugin
        // stays healthy, so a later call is not blocked by a false degrade.
        let status = host
            .list()
            .into_iter()
            .find(|s| s.id == "refusing")
            .unwrap();
        assert!(!status.degraded);
    }

    #[test]
    fn a_panicking_handle_is_caught_and_marks_the_plugin_degraded() {
        let dir = tmp_state_dir();
        let mut host =
            PluginHost::with_registry(dir, vec![Box::new(HandlePanics)]);
        host.enable(global("panic_handle")).unwrap();

        let err = host.handle("panic_handle.x", Value::Null).unwrap_err();
        assert_eq!(err.code, "plugin_panicked");

        // The daemon survived the panic and still answers list().
        let status = host
            .list()
            .into_iter()
            .find(|s| s.id == "panic_handle")
            .unwrap();
        assert!(status.degraded);
    }

    #[test]
    fn a_panicking_on_event_is_caught_and_marks_the_plugin_degraded() {
        let dir = tmp_state_dir();
        let mut host =
            PluginHost::with_registry(dir, vec![Box::new(EventPanics)]);
        host.enable(global("panic_event")).unwrap();

        // The fan-out must not unwind past the boundary.
        host.on_event(CoreEvent::Tick);

        let status = host
            .list()
            .into_iter()
            .find(|s| s.id == "panic_event")
            .unwrap();
        assert!(status.degraded);
    }

    #[test]
    fn a_disabled_plugin_is_not_fanned_an_event() {
        // EventPanics would panic (and degrade) if reached; disabled, it is
        // skipped, so it stays healthy.
        let dir = tmp_state_dir();
        let mut host =
            PluginHost::with_registry(dir, vec![Box::new(EventPanics)]);

        host.on_event(CoreEvent::Tick);

        let status = host
            .list()
            .into_iter()
            .find(|s| s.id == "panic_event")
            .unwrap();
        assert!(!status.degraded);
    }

    #[test]
    fn scope_maps_both_ways() {
        for scope in [ApiScope::Global, ApiScope::PerProject] {
            assert_eq!(api_scope(wire_scope(scope)), scope);
        }
    }
}
