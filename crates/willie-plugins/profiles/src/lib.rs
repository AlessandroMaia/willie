//! Configuration profiles applied per project.
//!
//! A profile is a versioned directory of fragments (settings, instructions,
//! rules, hooks, MCP servers). Applying one writes into the project and
//! into the harness state while preserving the existing files' format.
//!
//! Phase 1 (this crate's `handle`) creates a profile and lets its
//! fragments be read and written, each edit its own git commit inside the
//! profile's own repository at `<store_dir>/<name>/`. [`apply`] carries
//! Phase 2's format-preserving merge, pure over its inputs. `handle`
//! routes `profile.check`/`profile.apply` to it: `check` reads a
//! project's target files and plans the changes without writing; `apply`
//! additionally backs up every changing file under
//! `<workspace>/.willie-bak/<timestamp>/` and writes. Neither method
//! resolves a project id itself — the daemon fills the resolved
//! `_workspace` (and `_harness_settings`) into the request params before
//! `handle` ever runs (see `crates/willied/src/handlers.rs`'s
//! `profile_handle`), so this plugin never reaches into daemon state.

pub mod apply;
mod git;
mod model;

use std::{
    collections::HashMap,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;
use serde_json::{Value, json};
use willie_plugin_api::{
    Plugin, PluginCtx, PluginError, PluginManifest, PluginRequest,
    PluginResponse, Scope,
};

use apply::{Change, ChangeKind, FragmentContent, Targets};
use model::{
    Fragment, Profile, ProfileSummary, SettingsScope, is_valid_profile_name,
};

/// The profiles plugin.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProfilesPlugin;

/// Constructs the plugin for the daemon's registry.
#[must_use]
pub fn plugin() -> ProfilesPlugin {
    ProfilesPlugin
}

impl Plugin for ProfilesPlugin {
    fn manifest(&self) -> PluginManifest {
        // The id doubles as the method namespace the host routes on:
        // `profile.list`, `profile.create`, … all split to `profile`.
        PluginManifest {
            id: "profile",
            name: "Configuration profiles",
            scope: Scope::PerProject,
        }
    }

    fn handle(
        &mut self,
        ctx: &PluginCtx<'_>,
        req: PluginRequest,
    ) -> Result<PluginResponse, PluginError> {
        match req.method.as_str() {
            "profile.list" => list(ctx),
            "profile.create" => create(ctx, req.params),
            "profile.read_fragment" => read_fragment(ctx, req.params),
            "profile.write_fragment" => write_fragment(ctx, req.params),
            "profile.check" => check(ctx, req.params),
            "profile.apply" => apply_to_project(ctx, req.params),
            "profile.set_remote" => set_remote(ctx, req.params),
            "profile.push" => push(ctx, req.params),
            "profile.pull" => pull(ctx, req.params),
            other => Err(PluginError::BadRequest(format!(
                "unknown method `{other}`"
            ))),
        }
    }
}

// --------------------------------------------------------------- params

#[derive(Debug, Deserialize)]
struct CreateParams {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ReadFragmentParams {
    name: String,
    fragment: String,
}

#[derive(Debug, Deserialize)]
struct WriteFragmentParams {
    name: String,
    fragment: String,
    content: String,
}

/// `profile.check`'s params. `workspace`/`harness_settings` are never
/// supplied by a caller directly — they arrive as `_workspace`/
/// `_harness_settings`, filled in by the daemon's `profile_handle` seam
/// before this plugin ever runs (see the module doc). A bare `project_id`
/// with no matching `_workspace` is a caller wiring bug, so `workspace`
/// is required rather than optional: missing, it is `plugin_bad_request`,
/// distinct from `profile_target_missing` (a resolved workspace whose
/// directory is gone).
#[derive(Debug, Deserialize)]
struct CheckParams {
    name: String,
    #[serde(rename = "_workspace")]
    workspace: String,
    #[serde(rename = "_harness_settings", default)]
    harness_settings: Option<String>,
}

/// `profile.apply`'s params — the same shape as `CheckParams`; kept as
/// its own type rather than reused so the two methods' request shapes
/// can diverge without one accidentally affecting the other.
#[derive(Debug, Deserialize)]
struct ApplyParams {
    name: String,
    #[serde(rename = "_workspace")]
    workspace: String,
    #[serde(rename = "_harness_settings", default)]
    harness_settings: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SetRemoteParams {
    name: String,
    url: String,
}

/// `profile.push`'s params — its own type, not shared with
/// [`PullParams`], even though the shape is identical: the two methods'
/// request shapes should stay free to diverge independently, same
/// reasoning as `CheckParams`/`ApplyParams` above.
#[derive(Debug, Deserialize)]
struct PushParams {
    name: String,
}

#[derive(Debug, Deserialize)]
struct PullParams {
    name: String,
}

/// Parses `params` into `T`, or a `BadRequest` naming what did not fit.
fn parse_params<T: serde::de::DeserializeOwned>(
    params: Value,
) -> Result<T, PluginError> {
    serde_json::from_value(params)
        .map_err(|e| PluginError::BadRequest(format!("bad params: {e}")))
}

// -------------------------------------------------------------- methods

/// `profile.list`: every subdirectory of `store_dir` with a readable,
/// parseable `profile.toml`. A missing `store_dir` (nothing created yet)
/// or an entry that is not a profile directory is silently skipped rather
/// than refused — an empty list is a legitimate answer, not a fault.
fn list(ctx: &PluginCtx<'_>) -> Result<PluginResponse, PluginError> {
    let store = ctx.store_dir();
    let entries = match fs::read_dir(store) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return json_response(&Vec::<ProfileSummary>::new());
        }
        Err(e) => {
            return Err(PluginError::Internal(format!(
                "cannot read {}: {e}",
                store.display()
            )));
        }
    };

    let mut summaries = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| {
            PluginError::Internal(format!(
                "cannot read {}: {e}",
                store.display()
            ))
        })?;
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Some(profile) = try_read_profile(&entry.path()) else {
            continue;
        };
        summaries.push(ProfileSummary::from(&profile));
    }
    summaries.sort_by(|a, b| a.name.cmp(&b.name));

    json_response(&summaries)
}

/// `profile.create { name }`: scaffolds `<store_dir>/<name>/` with a
/// default `profile.toml`, an empty `settings.json`/`CLAUDE.md`, `git
/// init`s it and commits the scaffold.
fn create(
    ctx: &PluginCtx<'_>,
    params: Value,
) -> Result<PluginResponse, PluginError> {
    let CreateParams { name } = parse_params(params)?;
    let dir = profile_dir(ctx, &name)?;
    if dir.exists() {
        return Err(PluginError::coded(
            "profile_exists",
            format!("a profile named `{name}` already exists"),
            "pick a different name, or edit the existing profile",
        ));
    }

    let profile = Profile::scaffold(&name);
    fs::create_dir_all(dir.join("rules")).map_err(|e| io_fault(&dir, e))?;
    fs::create_dir_all(dir.join("hooks")).map_err(|e| io_fault(&dir, e))?;
    write_profile_toml(&dir, &profile)?;
    fs::write(dir.join("settings.json"), "{}\n")
        .map_err(|e| io_fault(&dir, e))?;
    fs::write(dir.join("CLAUDE.md"), "").map_err(|e| io_fault(&dir, e))?;

    git::init(&dir).map_err(git_fault)?;
    git::commit_all(&dir, "profile: scaffold").map_err(git_fault)?;

    json_response(&ProfileSummary::from(&profile))
}

/// `profile.read_fragment { name, fragment }`: the fragment file's
/// content, or an empty string for one that was scaffolded but never
/// written (e.g. `mcp` before its first `write_fragment`).
fn read_fragment(
    ctx: &PluginCtx<'_>,
    params: Value,
) -> Result<PluginResponse, PluginError> {
    let ReadFragmentParams { name, fragment } = parse_params(params)?;
    let dir = existing_profile_dir(ctx, &name)?;
    let frag = parse_fragment(&fragment)?;

    let path = dir.join(frag.relative_path());
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
        Err(e) => return Err(io_fault(&path, e)),
    };

    json_response(&json!({ "content": content }))
}

/// `profile.write_fragment { name, fragment, content }`: writes the
/// fragment file, marks it active in `profile.toml`, and commits both
/// changes together.
fn write_fragment(
    ctx: &PluginCtx<'_>,
    params: Value,
) -> Result<PluginResponse, PluginError> {
    let WriteFragmentParams {
        name,
        fragment,
        content,
    } = parse_params(params)?;
    let dir = existing_profile_dir(ctx, &name)?;
    let frag = parse_fragment(&fragment)?;

    let path = dir.join(frag.relative_path());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| io_fault(parent, e))?;
    }
    fs::write(&path, &content).map_err(|e| io_fault(&path, e))?;

    let mut profile = read_profile_toml(&dir)?;
    frag.mark_active(&mut profile.fragments);
    write_profile_toml(&dir, &profile)?;

    git::commit_all(&dir, &format!("profile: write {fragment}"))
        .map_err(git_fault)?;

    json_response(&json!({ "content": content }))
}

/// `profile.check { name, project_id }` (the daemon resolves `project_id`
/// to `_workspace`/`_harness_settings`, see the module doc): plans the
/// changes applying `name` would make, without writing anything.
fn check(
    ctx: &PluginCtx<'_>,
    params: Value,
) -> Result<PluginResponse, PluginError> {
    let CheckParams {
        name,
        workspace,
        harness_settings,
    } = parse_params(params)?;
    let dir = existing_profile_dir(ctx, &name)?;
    let profile = read_profile_toml(&dir)?;
    let workspace = PathBuf::from(workspace);
    let harness_settings = harness_settings.map(PathBuf::from);

    let changes = plan_project_changes(
        &dir,
        &profile,
        &workspace,
        harness_settings.as_deref(),
    )?;

    json_response(&json!({ "changes": changes }))
}

/// `profile.apply { name, project_id }`: plans the same changes `check`
/// would, then backs up every changing target under
/// `<workspace>/.willie-bak/<timestamp>/` and writes them. Named
/// `apply_to_project` (not `apply`, `apply.rs`'s own module name) purely
/// for readability at the call site — the two do not collide, `mod
/// apply` and a same-named `fn` live in different namespaces, but a
/// human skimming `handle`'s match arms should not have to know that.
fn apply_to_project(
    ctx: &PluginCtx<'_>,
    params: Value,
) -> Result<PluginResponse, PluginError> {
    let ApplyParams {
        name,
        workspace,
        harness_settings,
    } = parse_params(params)?;
    let dir = existing_profile_dir(ctx, &name)?;
    let profile = read_profile_toml(&dir)?;
    let workspace = PathBuf::from(workspace);
    let harness_settings = harness_settings.map(PathBuf::from);

    let changes = plan_project_changes(
        &dir,
        &profile,
        &workspace,
        harness_settings.as_deref(),
    )?;

    let backup_dir = backup_changes(&workspace, &changes)?;
    write_changes(&dir, &workspace, &changes)?;

    json_response(&json!({
        "changes": changes,
        "backup_path": backup_dir.to_string_lossy(),
    }))
}

// -------------------------------------------------------------- sync

/// `profile.set_remote { name, url }`: points the profile's own
/// repository at `url` as `origin`, adding it if this is the first time
/// or repointing it if one is already configured — the minimal sync's
/// only setup step.
fn set_remote(
    ctx: &PluginCtx<'_>,
    params: Value,
) -> Result<PluginResponse, PluginError> {
    let SetRemoteParams { name, url } = parse_params(params)?;
    let dir = existing_profile_dir(ctx, &name)?;
    git::set_remote(&dir, &url).map_err(git_fault)?;
    json_response(&json!({}))
}

/// `profile.push { name }`: `git push -u origin HEAD`, publishing the
/// profile's current history and recording the upstream so a later
/// `profile.pull` needs no branch name. No credential handling beyond
/// whatever the distribution's own `git` already has configured (an SSH
/// remote uses the session's own keys, out of scope here).
fn push(
    ctx: &PluginCtx<'_>,
    params: Value,
) -> Result<PluginResponse, PluginError> {
    let PushParams { name } = parse_params(params)?;
    let dir = existing_profile_dir(ctx, &name)?;
    git::push(&dir).map_err(git_fault)?;
    json_response(&json!({}))
}

/// `profile.pull { name }`: `git pull --ff-only`. A divergent history —
/// this machine and the remote each have commits the other lacks — has
/// no fast-forward to land, so it is refused as `profile_sync_conflict`
/// naming the profile, rather than left to git to attempt a merge that
/// could conflict inside the profile's own tracked files.
fn pull(
    ctx: &PluginCtx<'_>,
    params: Value,
) -> Result<PluginResponse, PluginError> {
    let PullParams { name } = parse_params(params)?;
    let dir = existing_profile_dir(ctx, &name)?;
    match git::pull(&dir) {
        Ok(()) => json_response(&json!({})),
        Err(e) if e.code == "pull_conflict" => Err(sync_conflict(&name)),
        Err(e) => Err(git_fault(e)),
    }
}

// ----------------------------------------------------- check / apply

/// Reads a profile's active fragments and the corresponding target
/// files, then runs [`apply::plan_changes`] to compute what applying
/// would do. Shared by `check` (which stops here) and `apply_to_project`
/// (which additionally backs up and writes). A missing `workspace`
/// directory is `profile_target_missing`, checked before anything is
/// read — neither `check` nor `apply_to_project` can answer meaningfully
/// against a project whose ext4 clone is gone. A `settings` fragment
/// marked `scope = "global"` in `profile.toml` additionally plans a
/// second change against the harness state's own `settings.json`
/// (`harness_settings`), independent of the project's — each keeps its
/// own existing key order.
fn plan_project_changes(
    profile_dir: &Path,
    profile: &Profile,
    workspace: &Path,
    harness_settings: Option<&Path>,
) -> Result<Vec<Change>, PluginError> {
    if !workspace.is_dir() {
        return Err(target_missing(workspace));
    }

    let settings_content = read_fragment_pair(
        profile_dir,
        workspace,
        "settings.json",
        ".claude/settings.json",
        profile.fragments.settings,
    )?;
    let mcp_content = read_fragment_pair(
        profile_dir,
        workspace,
        "mcp.json",
        ".claude/settings.json",
        profile.fragments.mcp,
    )?;
    let instructions_content = read_fragment_pair(
        profile_dir,
        workspace,
        "CLAUDE.md",
        "CLAUDE.md",
        profile.fragments.instructions,
    )?;
    let rules = read_file_family(
        profile_dir,
        workspace,
        "rules",
        &profile.fragments.rules,
    )?;
    let hooks = read_file_family(
        profile_dir,
        workspace,
        "hooks",
        &profile.fragments.hooks,
    )?;

    let targets = Targets {
        settings: settings_content.clone(),
        mcp: mcp_content.clone(),
        instructions: instructions_content,
        rules,
        hooks,
    };
    let mut changes = apply::plan_changes(profile, &targets)
        .map_err(apply_error_to_plugin)?;

    if profile.fragments.settings
        && profile.fragments.settings_scope == SettingsScope::Global
        && let Some(harness_path) = harness_settings
        && let Some(change) = plan_harness_settings_change(
            profile,
            settings_content,
            mcp_content,
            harness_path,
        )?
    {
        changes.push(change);
    }

    Ok(changes)
}

/// The second, harness-scoped `Change` a `scope = "global"` `settings`
/// fragment plans: the same `settings`/`mcp` fragment content the
/// project's own change used, merged instead against the harness
/// state's own existing `settings.json` — a different destination can
/// have different existing content and key order, so this is not simply
/// a copy of the project's change. Reuses [`apply::plan_changes`] with a
/// profile mask that turns off every fragment but `settings`/`mcp` (the
/// only two that ever target `.claude/settings.json`), so the harness
/// destination is never handed a rule or hook file it has no directory
/// for. The resulting `Change`'s `path` is overwritten with
/// `harness_path` itself (an absolute path) — `apply_to_project`'s
/// `resolve_dest` tells an absolute destination from the project's
/// relative ones by that alone.
fn plan_harness_settings_change(
    profile: &Profile,
    settings_content: Option<FragmentContent>,
    mcp_content: Option<FragmentContent>,
    harness_path: &Path,
) -> Result<Option<Change>, PluginError> {
    let harness_existing = read_optional(harness_path)?;
    let mut harness_profile = profile.clone();
    harness_profile.fragments.instructions = false;
    harness_profile.fragments.rules.clear();
    harness_profile.fragments.hooks.clear();

    let harness_targets = Targets {
        settings: settings_content.map(|c| FragmentContent {
            fragment: c.fragment,
            existing: harness_existing.clone(),
        }),
        mcp: mcp_content.map(|c| FragmentContent {
            fragment: c.fragment,
            existing: harness_existing,
        }),
        ..Targets::default()
    };
    let mut harness_changes =
        apply::plan_changes(&harness_profile, &harness_targets)
            .map_err(apply_error_to_plugin)?;
    let Some(mut change) = harness_changes.pop() else {
        return Ok(None);
    };
    change.path = harness_path.to_string_lossy().into_owned();
    Ok(Some(change))
}

/// One fragment's inputs when `active` — the fragment file under
/// `profile_dir` (an unwritten-but-active fragment reads as empty, same
/// as `read_fragment`) and the existing content at
/// `workspace.join(target_rel)`. `None` when the fragment is not active:
/// `plan_changes` never looks at it, so there is nothing to read.
fn read_fragment_pair(
    profile_dir: &Path,
    workspace: &Path,
    fragment_file: &str,
    target_rel: &str,
    active: bool,
) -> Result<Option<FragmentContent>, PluginError> {
    if !active {
        return Ok(None);
    }
    let fragment = read_required(&profile_dir.join(fragment_file))?;
    let existing = read_optional(&workspace.join(target_rel))?;
    Ok(Some(FragmentContent { fragment, existing }))
}

/// Every active `rules`/`hooks` file's inputs, keyed by file name — the
/// same shape `apply::Targets::rules`/`hooks` expect.
fn read_file_family(
    profile_dir: &Path,
    workspace: &Path,
    family: &str,
    names: &[String],
) -> Result<HashMap<String, FragmentContent>, PluginError> {
    let mut map = HashMap::new();
    for name in names {
        let fragment = read_required(&profile_dir.join(family).join(name))?;
        let existing =
            read_optional(&workspace.join(".claude").join(family).join(name))?;
        map.insert(name.clone(), FragmentContent { fragment, existing });
    }
    Ok(map)
}

/// A file's content, or an empty string if it does not exist yet — a
/// profile fragment marked active in `profile.toml` whose file was
/// somehow never written reads the same way `read_fragment` treats it.
fn read_required(path: &Path) -> Result<String, PluginError> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(io_fault(path, e)),
    }
}

/// A target file's content, or `None` if it does not exist yet (the
/// change plans as a `Create` rather than a `Merge`/`Overwrite`).
fn read_optional(path: &Path) -> Result<Option<String>, PluginError> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_fault(path, e)),
    }
}

/// Backs up every changing target's *current* content (a `Merge`/
/// `Overwrite`; a `Create` has nothing to back up yet) into
/// `<workspace>/.willie-bak/<timestamp>/`, before any write — a
/// differential backup, not a full snapshot. Returns the backup
/// directory even when nothing was actually copied (every change was a
/// `Create`): `apply_to_project` still reports a `backup_path`, and an
/// empty directory is an honest answer to "what did applying change".
fn backup_changes(
    workspace: &Path,
    changes: &[Change],
) -> Result<PathBuf, PluginError> {
    let backup_root = workspace.join(".willie-bak").join(backup_timestamp());
    for change in changes {
        if change.kind == ChangeKind::Create {
            continue;
        }
        let dest = resolve_dest(workspace, &change.path);
        let backup_dest = backup_root.join(backup_relative_path(&change.path));
        if let Some(parent) = backup_dest.parent() {
            fs::create_dir_all(parent).map_err(|e| io_fault(parent, e))?;
        }
        match fs::read(&dest) {
            Ok(bytes) => fs::write(&backup_dest, bytes)
                .map_err(|e| io_fault(&backup_dest, e))?,
            // Changed on disk since planning: nothing to back up.
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(io_fault(&dest, e)),
        }
    }
    Ok(backup_root)
}

/// Writes every change's `after` content to its destination, creating
/// parent directories as needed. A hook file's executable bit is
/// best-effort copied from the profile's own fragment file afterwards —
/// its content is already correct either way, so a failure to copy the
/// mode is not surfaced as a fault.
fn write_changes(
    profile_dir: &Path,
    workspace: &Path,
    changes: &[Change],
) -> Result<(), PluginError> {
    for change in changes {
        let dest = resolve_dest(workspace, &change.path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| io_fault(parent, e))?;
        }
        fs::write(&dest, &change.after).map_err(|e| io_fault(&dest, e))?;
        if let Some(name) = change.path.strip_prefix(".claude/hooks/") {
            copy_hook_mode(&profile_dir.join("hooks").join(name), &dest);
        }
    }
    Ok(())
}

#[cfg(unix)]
fn copy_hook_mode(source: &Path, dest: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = fs::metadata(source) {
        let _ = fs::set_permissions(
            dest,
            fs::Permissions::from_mode(meta.permissions().mode()),
        );
    }
}

#[cfg(not(unix))]
fn copy_hook_mode(_source: &Path, _dest: &Path) {}

/// A change's destination: `workspace.join(path)` for a project-relative
/// path, or `path` itself when it is absolute (the harness-scoped
/// `settings.json`, outside the workspace entirely).
fn resolve_dest(workspace: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace.join(p)
    }
}

/// Where a change's *prior* content backs up to, relative to the backup
/// root: the change's own relative path, mirrored — except an absolute
/// path (the harness-scoped settings.json) backs up under a fixed name,
/// since there is at most one such destination and its own path carries
/// no meaningful relative structure under the workspace.
fn backup_relative_path(path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        PathBuf::from("harness-settings.json")
    } else {
        p.to_path_buf()
    }
}

/// A monotonic-enough backup folder name: decimal nanoseconds since the
/// Unix epoch. Plain `std`, no date-time dependency; not meant to be
/// read as a calendar date, only to keep repeated applies from
/// collapsing into the same backup directory.
fn backup_timestamp() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}{:09}", dur.as_secs(), dur.subsec_nanos())
}

/// Maps a pure [`apply::ApplyError`] onto the plugin's coded error,
/// preserving its code and message and adding a remediation per code —
/// `apply.rs` has no business knowing about `PluginError`'s shape.
fn apply_error_to_plugin(e: apply::ApplyError) -> PluginError {
    let remediation = match e.code {
        "profile_fragment_invalid" => {
            "fix the fragment's or the target's content so it parses as \
             JSON, then check or apply again"
        }
        "profile_markers_malformed" => {
            "fix or remove the stray <!-- willie:begin --> marker in the \
             target file, then apply again"
        }
        "profile_fragment_missing" => {
            "this is a wiring bug in Willie, not something to fix in the \
             profile; check the daemon log"
        }
        _ => "check the daemon log for details",
    };
    PluginError::coded(e.code, e.message, remediation)
}

fn target_missing(workspace: &Path) -> PluginError {
    PluginError::coded(
        "profile_target_missing",
        format!(
            "the project workspace {} does not exist",
            workspace.display()
        ),
        "the project's ext4 workspace is gone; re-add the project before \
         checking or applying a profile",
    )
}

// --------------------------------------------------------------- helpers

/// `<store_dir>/<name>/`, not necessarily existing yet. Refuses
/// `profile_name_invalid` for a name the grammar rejects (see
/// `is_valid_profile_name`) *before* building any path from it, and — as
/// a second, independent line of defense — refuses the same code if the
/// joined path is somehow not actually a descendant of `store_dir`
/// anyway. The second check exists because a name-level rule can only
/// reject shapes it was written to anticipate; `Path::join` itself can
/// behave in ways that quietly step outside the base (a Windows
/// drive-letter argument replaces the base outright rather than
/// nesting under it), so the result is verified after the fact rather
/// than trusted.
fn profile_dir(
    ctx: &PluginCtx<'_>,
    name: &str,
) -> Result<PathBuf, PluginError> {
    if !is_valid_profile_name(name) {
        return Err(invalid_name(name));
    }
    let store = ctx.store_dir();
    let dir = store.join(name);
    if !is_contained(store, &dir) {
        return Err(invalid_name(name));
    }
    Ok(dir)
}

/// Whether `dir` is still a descendant of `store` — the containment
/// check `profile_dir` runs after joining, independent of whatever the
/// name grammar already ruled out.
fn is_contained(store: &Path, dir: &Path) -> bool {
    dir.strip_prefix(store).is_ok()
}

/// Resolves `name` to an existing profile's directory. `profile_dir`
/// already answers `profile_name_invalid` for a name that could never be
/// valid or whose joined path would escape `store_dir`; what is left
/// here is a *valid* name with no `profile.toml` there, which is
/// `profile_not_found`.
fn existing_profile_dir(
    ctx: &PluginCtx<'_>,
    name: &str,
) -> Result<PathBuf, PluginError> {
    let dir = profile_dir(ctx, name)?;
    if !dir.join("profile.toml").is_file() {
        return Err(not_found(name));
    }
    Ok(dir)
}

/// Reads `<dir>/profile.toml` if present and parseable; `None` for
/// anything else (a stray non-profile directory under `store_dir`), used
/// by `list` to skip rather than fail on it.
fn try_read_profile(dir: &Path) -> Option<Profile> {
    let text = fs::read_to_string(dir.join("profile.toml")).ok()?;
    toml::from_str(&text).ok()
}

fn read_profile_toml(dir: &Path) -> Result<Profile, PluginError> {
    let path = dir.join("profile.toml");
    let text = fs::read_to_string(&path).map_err(|e| io_fault(&path, e))?;
    toml::from_str(&text).map_err(|e| {
        PluginError::Internal(format!("cannot parse {}: {e}", path.display()))
    })
}

fn write_profile_toml(
    dir: &Path,
    profile: &Profile,
) -> Result<(), PluginError> {
    let path = dir.join("profile.toml");
    let text = toml::to_string_pretty(profile).map_err(|e| {
        PluginError::Internal(format!("cannot encode profile.toml: {e}"))
    })?;
    fs::write(&path, text).map_err(|e| io_fault(&path, e))
}

fn parse_fragment(s: &str) -> Result<Fragment, PluginError> {
    Fragment::parse(s).ok_or_else(|| {
        PluginError::coded(
            "profile_fragment_unknown",
            format!("`{s}` is not a known fragment"),
            "use one of settings, instructions, mcp, rules/<file>, \
             hooks/<file>",
        )
    })
}

fn invalid_name(name: &str) -> PluginError {
    PluginError::coded(
        "profile_name_invalid",
        format!("`{name}` is not a valid profile name"),
        "use a name with no path separators, and not `.` or `..`",
    )
}

fn not_found(name: &str) -> PluginError {
    PluginError::coded(
        "profile_not_found",
        format!("no profile named `{name}`"),
        "check profile.list for the available profile names",
    )
}

/// `profile.pull` hit a non-fast-forward or conflict: this machine and
/// the remote have each moved on independently, so nothing was changed.
fn sync_conflict(name: &str) -> PluginError {
    PluginError::coded(
        "profile_sync_conflict",
        format!(
            "profile `{name}` has diverged from its remote and cannot be \
             fast-forwarded"
        ),
        "resolve it in a terminal inside the distribution, then pull again",
    )
}

/// An unexpected filesystem failure — not attributable to the caller's
/// request, so it is a genuine fault rather than a coded refusal.
fn io_fault(path: &Path, e: std::io::Error) -> PluginError {
    PluginError::Internal(format!("cannot access {}: {e}", path.display()))
}

/// A `git` failure: also a genuine fault, since every request-shaped
/// reason to fail (an unknown profile, an invalid fragment) is refused
/// before git is ever invoked.
fn git_fault(e: git::GitError) -> PluginError {
    PluginError::Internal(format!("git {}: {}", e.code, e.message))
}

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
    use willie_plugin_api::PluginEmission;

    use super::*;

    /// A unique, empty scratch directory under the system temp root, to
    /// build a `PluginCtx` over in the test that uses it — kept as its
    /// own step (not bundled with the `PluginCtx`) because the context
    /// borrows from it and a helper cannot return a self-referential pair.
    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "willie-profiles-plugin-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A unique, empty scratch directory standing in for a project's ext4
    /// workspace — `check`/`apply` read and write `.claude/`, `CLAUDE.md`
    /// and `.willie-bak/` under it, distinct from the profile store dir
    /// above.
    fn workspace_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "willie-profiles-workspace-{name}-{}",
            std::process::id()
        ));
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

    fn ok_value(result: Result<PluginResponse, PluginError>) -> Value {
        result.unwrap().into_value()
    }

    #[test]
    fn profiles_are_scoped_per_project() {
        assert_eq!(ProfilesPlugin.manifest().scope, Scope::PerProject);
    }

    #[test]
    fn create_scaffolds_a_profile_toml_and_an_initial_commit() {
        let dir = scratch_dir("create");
        let ctx = PluginCtx::new(&dir, &noop_emit);
        let mut plugin = ProfilesPlugin;

        let resp = plugin
            .handle(&ctx, req("profile.create", json!({"name": "funcef-auth"})))
            .unwrap();
        assert_eq!(
            resp.into_value(),
            json!({"name": "funcef-auth", "fragments_active": []})
        );

        let profile_dir = dir.join("funcef-auth");
        assert!(profile_dir.join(".git").exists());
        assert!(profile_dir.join("profile.toml").is_file());
        assert!(profile_dir.join("settings.json").is_file());
        assert!(profile_dir.join("CLAUDE.md").is_file());

        let log = git::run(&profile_dir, &["log", "--oneline"]).unwrap();
        assert_eq!(log.lines().count(), 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn creating_the_same_name_twice_is_profile_exists() {
        let dir = scratch_dir("dup");
        let ctx = PluginCtx::new(&dir, &noop_emit);
        let mut plugin = ProfilesPlugin;
        plugin
            .handle(&ctx, req("profile.create", json!({"name": "x"})))
            .unwrap();

        let err = plugin
            .handle(&ctx, req("profile.create", json!({"name": "x"})))
            .unwrap_err();
        assert_eq!(err.code(), "profile_exists");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_refuses_a_name_with_a_path_separator() {
        let dir = scratch_dir("badname");
        let ctx = PluginCtx::new(&dir, &noop_emit);
        let mut plugin = ProfilesPlugin;

        let err = plugin
            .handle(&ctx, req("profile.create", json!({"name": "a/../b"})))
            .unwrap_err();
        assert_eq!(err.code(), "profile_name_invalid");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_refuses_a_windows_drive_letter_name_and_touches_nothing() {
        // On Windows, `PathBuf::join` with a drive-prefixed argument
        // replaces the base path outright rather than nesting under it —
        // `store_dir().join("C:foo")` would land outside `store_dir`
        // entirely. This crate carries no `cfg(target_os = "linux")` of
        // its own, so `cargo test --workspace` runs it under real
        // Windows path semantics on a Windows development machine; the
        // name must be refused before any path is built from it, and
        // nothing under the store dir (the only place this test is
        // allowed to touch) may appear as a side effect.
        let dir = scratch_dir("drive-letter");
        let ctx = PluginCtx::new(&dir, &noop_emit);
        let mut plugin = ProfilesPlugin;

        for name in ["C:foo", "a:bar"] {
            let err = plugin
                .handle(&ctx, req("profile.create", json!({"name": name})))
                .unwrap_err();
            assert_eq!(err.code(), "profile_name_invalid", "name `{name}`");
        }

        // Nothing was created under the store dir either.
        let entries: Vec<_> = fs::read_dir(&dir).unwrap().collect();
        assert!(entries.is_empty(), "store dir gained an entry: {entries:?}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_containment_escape_is_refused_even_past_the_name_grammar() {
        // Independent of `is_valid_profile_name`: even a `dir` that a
        // join produced outside `store_dir` by some mechanism the name
        // grammar did not anticipate must still be refused, never
        // silently used. Exercises the containment check directly with
        // synthesised paths, portable to every OS this crate's tests run
        // on (a real Windows-only drive-letter join is covered end to
        // end above, on Windows).
        let store = Path::new("/store/profile/x");
        let escaped = Path::new("C:foo");
        assert!(!is_contained(store, escaped));

        let contained = store.join("settings.json");
        assert!(is_contained(store, &contained));
    }

    #[test]
    fn list_reports_the_created_profile() {
        let dir = scratch_dir("list");
        let ctx = PluginCtx::new(&dir, &noop_emit);
        let mut plugin = ProfilesPlugin;
        plugin
            .handle(&ctx, req("profile.create", json!({"name": "a"})))
            .unwrap();
        plugin
            .handle(&ctx, req("profile.create", json!({"name": "b"})))
            .unwrap();

        let listed =
            ok_value(plugin.handle(&ctx, req("profile.list", Value::Null)));
        assert_eq!(
            listed,
            json!([
                {"name": "a", "fragments_active": []},
                {"name": "b", "fragments_active": []},
            ])
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn listing_an_empty_store_dir_is_an_empty_list() {
        let dir = scratch_dir("empty");
        let ctx = PluginCtx::new(&dir, &noop_emit);
        let listed = ok_value(
            ProfilesPlugin.handle(&ctx, req("profile.list", Value::Null)),
        );
        assert_eq!(listed, json!([]));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_then_read_fragment_round_trips_and_commits() {
        let dir = scratch_dir("fragment");
        let ctx = PluginCtx::new(&dir, &noop_emit);
        let mut plugin = ProfilesPlugin;
        plugin
            .handle(&ctx, req("profile.create", json!({"name": "x"})))
            .unwrap();

        let written = ok_value(plugin.handle(
            &ctx,
            req(
                "profile.write_fragment",
                json!({"name": "x", "fragment": "settings", "content": "{\"a\":1}"}),
            ),
        ));
        assert_eq!(written, json!({"content": "{\"a\":1}"}));

        let read = ok_value(plugin.handle(
            &ctx,
            req(
                "profile.read_fragment",
                json!({"name": "x", "fragment": "settings"}),
            ),
        ));
        assert_eq!(read, json!({"content": "{\"a\":1}"}));

        let profile_dir = dir.join("x");
        let log = git::run(&profile_dir, &["log", "--oneline"]).unwrap();
        assert_eq!(log.lines().count(), 2); // scaffold + this write

        // The fragment is now reported active by `list`.
        let listed =
            ok_value(plugin.handle(&ctx, req("profile.list", Value::Null)));
        assert_eq!(
            listed,
            json!([{"name": "x", "fragments_active": ["settings"]}])
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_fragment_on_a_rule_file_tracks_it_in_profile_toml() {
        let dir = scratch_dir("rule");
        let ctx = PluginCtx::new(&dir, &noop_emit);
        let mut plugin = ProfilesPlugin;
        plugin
            .handle(&ctx, req("profile.create", json!({"name": "x"})))
            .unwrap();

        plugin
            .handle(
                &ctx,
                req(
                    "profile.write_fragment",
                    json!({
                        "name": "x",
                        "fragment": "rules/no-force-push.md",
                        "content": "no force push"
                    }),
                ),
            )
            .unwrap();

        assert!(dir.join("x/rules/no-force-push.md").is_file());
        let listed =
            ok_value(plugin.handle(&ctx, req("profile.list", Value::Null)));
        assert_eq!(
            listed,
            json!([{
                "name": "x",
                "fragments_active": ["rules/no-force-push.md"],
            }])
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_fragment_on_an_unwritten_fragment_is_empty() {
        let dir = scratch_dir("unwritten");
        let ctx = PluginCtx::new(&dir, &noop_emit);
        let mut plugin = ProfilesPlugin;
        plugin
            .handle(&ctx, req("profile.create", json!({"name": "x"})))
            .unwrap();

        let read = ok_value(plugin.handle(
            &ctx,
            req(
                "profile.read_fragment",
                json!({"name": "x", "fragment": "mcp"}),
            ),
        ));
        assert_eq!(read, json!({"content": ""}));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_fragment_is_refused() {
        let dir = scratch_dir("unknown-fragment");
        let ctx = PluginCtx::new(&dir, &noop_emit);
        let mut plugin = ProfilesPlugin;
        plugin
            .handle(&ctx, req("profile.create", json!({"name": "x"})))
            .unwrap();

        let err = plugin
            .handle(
                &ctx,
                req(
                    "profile.read_fragment",
                    json!({"name": "x", "fragment": "nope"}),
                ),
            )
            .unwrap_err();
        assert_eq!(err.code(), "profile_fragment_unknown");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_profile_is_refused_on_read_and_write() {
        let dir = scratch_dir("unknown-profile");
        let ctx = PluginCtx::new(&dir, &noop_emit);
        let mut plugin = ProfilesPlugin;

        let err = plugin
            .handle(
                &ctx,
                req(
                    "profile.read_fragment",
                    json!({"name": "ghost", "fragment": "settings"}),
                ),
            )
            .unwrap_err();
        assert_eq!(err.code(), "profile_not_found");

        let err = plugin
            .handle(
                &ctx,
                req(
                    "profile.write_fragment",
                    json!({"name": "ghost", "fragment": "settings", "content": "x"}),
                ),
            )
            .unwrap_err();
        assert_eq!(err.code(), "profile_not_found");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_path_traversal_name_is_refused_as_invalid_not_a_fault() {
        // `profile_dir` (shared by `create` and `existing_profile_dir`)
        // rejects a name the grammar could never accept before it ever
        // resolves to a path, so this is `profile_name_invalid`, distinct
        // from `profile_not_found` (a valid name with no profile there).
        let dir = scratch_dir("traversal");
        let ctx = PluginCtx::new(&dir, &noop_emit);
        let err = ProfilesPlugin
            .handle(
                &ctx,
                req(
                    "profile.read_fragment",
                    json!({"name": "../../etc", "fragment": "settings"}),
                ),
            )
            .unwrap_err();
        assert_eq!(err.code(), "profile_name_invalid");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_params_shape_is_a_bad_request() {
        let dir = scratch_dir("bad-params");
        let ctx = PluginCtx::new(&dir, &noop_emit);
        let err = ProfilesPlugin
            .handle(&ctx, req("profile.create", json!({"not_name": 1})))
            .unwrap_err();
        assert_eq!(err.code(), "plugin_bad_request");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_method_is_a_bad_request() {
        let dir = scratch_dir("bad-method");
        let ctx = PluginCtx::new(&dir, &noop_emit);
        let err = ProfilesPlugin
            .handle(&ctx, req("profile.frobnicate", Value::Null))
            .unwrap_err();
        assert_eq!(err.code(), "plugin_bad_request");
        let _ = fs::remove_dir_all(&dir);
    }

    // ------------------------------------------- profile.check / apply

    #[test]
    fn check_lists_changes_without_writing_anything() {
        let dir = scratch_dir("check");
        let ws = workspace_dir("check-ws");
        let ctx = PluginCtx::new(&dir, &noop_emit);
        let mut plugin = ProfilesPlugin;
        plugin
            .handle(&ctx, req("profile.create", json!({"name": "x"})))
            .unwrap();
        plugin
            .handle(
                &ctx,
                req(
                    "profile.write_fragment",
                    json!({
                        "name": "x",
                        "fragment": "settings",
                        "content": "{\"a\": 1}"
                    }),
                ),
            )
            .unwrap();

        let resp = ok_value(plugin.handle(
            &ctx,
            req(
                "profile.check",
                json!({"name": "x", "_workspace": ws.to_string_lossy()}),
            ),
        ));
        let changes = resp["changes"].as_array().unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0]["path"], ".claude/settings.json");
        assert_eq!(changes[0]["kind"], "create");

        // Nothing was written: `check` only previews.
        assert!(!ws.join(".claude").exists());

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn apply_writes_settings_claude_md_and_a_rule_and_backs_up_the_prior_files()
    {
        let dir = scratch_dir("apply");
        let ws = workspace_dir("apply-ws");
        fs::create_dir_all(ws.join(".claude/rules")).unwrap();
        fs::write(ws.join(".claude/settings.json"), "{\"b\": 1, \"a\": 2}\n")
            .unwrap();
        fs::write(ws.join("CLAUDE.md"), "# Notes\n\nprose\n").unwrap();
        fs::write(ws.join(".claude/rules/no-force-push.md"), "old rule\n")
            .unwrap();

        let ctx = PluginCtx::new(&dir, &noop_emit);
        let mut plugin = ProfilesPlugin;
        plugin
            .handle(&ctx, req("profile.create", json!({"name": "x"})))
            .unwrap();
        plugin
            .handle(
                &ctx,
                req(
                    "profile.write_fragment",
                    json!({
                        "name": "x",
                        "fragment": "settings",
                        "content": "{\"a\": 20, \"c\": 3}"
                    }),
                ),
            )
            .unwrap();
        plugin
            .handle(
                &ctx,
                req(
                    "profile.write_fragment",
                    json!({
                        "name": "x",
                        "fragment": "instructions",
                        "content": "new"
                    }),
                ),
            )
            .unwrap();
        plugin
            .handle(
                &ctx,
                req(
                    "profile.write_fragment",
                    json!({
                        "name": "x",
                        "fragment": "rules/no-force-push.md",
                        "content": "new rule\n"
                    }),
                ),
            )
            .unwrap();

        let resp = ok_value(plugin.handle(
            &ctx,
            req(
                "profile.apply",
                json!({"name": "x", "_workspace": ws.to_string_lossy()}),
            ),
        ));

        // settings.json is merged and keeps the target's key order: `b`
        // first (untouched), then `a` (updated in place), then `c`
        // (appended).
        let settings =
            fs::read_to_string(ws.join(".claude/settings.json")).unwrap();
        let b_pos = settings.find("\"b\"").unwrap();
        let a_pos = settings.find("\"a\"").unwrap();
        let c_pos = settings.find("\"c\"").unwrap();
        assert!(b_pos < a_pos && a_pos < c_pos, "settings: {settings}");

        // CLAUDE.md is merged between the markers; the prose is kept.
        let claude_md = fs::read_to_string(ws.join("CLAUDE.md")).unwrap();
        assert!(claude_md.starts_with("# Notes\n\nprose\n"), "{claude_md}");
        assert!(
            claude_md
                .contains("<!-- willie:begin -->\nnew\n<!-- willie:end -->"),
            "{claude_md}"
        );

        // The rule file was overwritten with the fragment's content.
        let rule =
            fs::read_to_string(ws.join(".claude/rules/no-force-push.md"))
                .unwrap();
        assert_eq!(rule, "new rule\n");

        // A backup was made of every prior file, under the workspace.
        let backup_path = resp["backup_path"].as_str().unwrap();
        let backup_dir = PathBuf::from(backup_path);
        assert!(backup_dir.starts_with(&ws), "{backup_path}");
        assert_eq!(
            fs::read_to_string(backup_dir.join(".claude/settings.json"))
                .unwrap(),
            "{\"b\": 1, \"a\": 2}\n"
        );
        assert_eq!(
            fs::read_to_string(backup_dir.join("CLAUDE.md")).unwrap(),
            "# Notes\n\nprose\n"
        );
        assert_eq!(
            fs::read_to_string(
                backup_dir.join(".claude/rules/no-force-push.md")
            )
            .unwrap(),
            "old rule\n"
        );

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn a_second_apply_is_idempotent_where_the_fragment_already_merged() {
        let dir = scratch_dir("apply-twice");
        let ws = workspace_dir("apply-twice-ws");
        let ctx = PluginCtx::new(&dir, &noop_emit);
        let mut plugin = ProfilesPlugin;
        plugin
            .handle(&ctx, req("profile.create", json!({"name": "x"})))
            .unwrap();
        plugin
            .handle(
                &ctx,
                req(
                    "profile.write_fragment",
                    json!({
                        "name": "x",
                        "fragment": "instructions",
                        "content": "new"
                    }),
                ),
            )
            .unwrap();

        let params = json!({"name": "x", "_workspace": ws.to_string_lossy()});
        plugin
            .handle(&ctx, req("profile.apply", params.clone()))
            .unwrap();
        let first = fs::read_to_string(ws.join("CLAUDE.md")).unwrap();

        plugin.handle(&ctx, req("profile.apply", params)).unwrap();
        let second = fs::read_to_string(ws.join("CLAUDE.md")).unwrap();

        assert_eq!(first, second);

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn an_invalid_fragment_refuses_check_and_apply_before_any_write() {
        let dir = scratch_dir("invalid");
        let ws = workspace_dir("invalid-ws");
        let ctx = PluginCtx::new(&dir, &noop_emit);
        let mut plugin = ProfilesPlugin;
        plugin
            .handle(&ctx, req("profile.create", json!({"name": "x"})))
            .unwrap();
        plugin
            .handle(
                &ctx,
                req(
                    "profile.write_fragment",
                    json!({
                        "name": "x",
                        "fragment": "settings",
                        "content": "not json"
                    }),
                ),
            )
            .unwrap();
        let params = json!({"name": "x", "_workspace": ws.to_string_lossy()});

        let err = plugin
            .handle(&ctx, req("profile.check", params.clone()))
            .unwrap_err();
        assert_eq!(err.code(), "profile_fragment_invalid");

        let err = plugin
            .handle(&ctx, req("profile.apply", params))
            .unwrap_err();
        assert_eq!(err.code(), "profile_fragment_invalid");

        // Neither call wrote anything to the workspace.
        assert!(!ws.join(".claude").exists());

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn a_missing_workspace_is_profile_target_missing_for_check_and_apply() {
        let dir = scratch_dir("missing-ws");
        let ctx = PluginCtx::new(&dir, &noop_emit);
        let mut plugin = ProfilesPlugin;
        plugin
            .handle(&ctx, req("profile.create", json!({"name": "x"})))
            .unwrap();

        let ghost_ws = std::env::temp_dir()
            .join(format!("willie-profiles-ghost-{}", std::process::id()));
        let _ = fs::remove_dir_all(&ghost_ws);
        let params =
            json!({"name": "x", "_workspace": ghost_ws.to_string_lossy()});

        let err = plugin
            .handle(&ctx, req("profile.check", params.clone()))
            .unwrap_err();
        assert_eq!(err.code(), "profile_target_missing");

        let err = plugin
            .handle(&ctx, req("profile.apply", params))
            .unwrap_err();
        assert_eq!(err.code(), "profile_target_missing");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_global_scoped_settings_fragment_also_applies_to_the_harness_settings()
    {
        let dir = scratch_dir("global-scope");
        let ws = workspace_dir("global-scope-ws");
        let harness_dir = workspace_dir("global-scope-harness");
        let harness_settings = harness_dir.join("settings.json");
        fs::write(&harness_settings, "{\"z\": 9}\n").unwrap();

        let ctx = PluginCtx::new(&dir, &noop_emit);
        let mut plugin = ProfilesPlugin;
        plugin
            .handle(&ctx, req("profile.create", json!({"name": "x"})))
            .unwrap();
        plugin
            .handle(
                &ctx,
                req(
                    "profile.write_fragment",
                    json!({
                        "name": "x",
                        "fragment": "settings",
                        "content": "{\"theme\": \"dark\"}"
                    }),
                ),
            )
            .unwrap();
        // Mark the settings fragment global-scoped directly, as the app's
        // profile editor would.
        let toml_path = dir.join("x/profile.toml");
        let mut profile: Profile =
            toml::from_str(&fs::read_to_string(&toml_path).unwrap()).unwrap();
        profile.fragments.settings_scope = SettingsScope::Global;
        fs::write(&toml_path, toml::to_string_pretty(&profile).unwrap())
            .unwrap();

        let resp = ok_value(plugin.handle(
            &ctx,
            req(
                "profile.apply",
                json!({
                    "name": "x",
                    "_workspace": ws.to_string_lossy(),
                    "_harness_settings": harness_settings.to_string_lossy(),
                }),
            ),
        ));

        let changes = resp["changes"].as_array().unwrap();
        assert_eq!(changes.len(), 2, "{changes:?}");

        let harness_written = fs::read_to_string(&harness_settings).unwrap();
        let value: Value = serde_json::from_str(&harness_written).unwrap();
        assert_eq!(value["theme"], "dark");
        assert_eq!(value["z"], 9);

        // The harness settings' prior content was also backed up, under
        // the workspace's own backup directory.
        let backup_path = resp["backup_path"].as_str().unwrap();
        let backed_up = fs::read_to_string(
            PathBuf::from(backup_path).join("harness-settings.json"),
        )
        .unwrap();
        assert_eq!(backed_up, "{\"z\": 9}\n");

        let project_written =
            fs::read_to_string(ws.join(".claude/settings.json")).unwrap();
        let value: Value = serde_json::from_str(&project_written).unwrap();
        assert_eq!(value["theme"], "dark");

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&ws);
        let _ = fs::remove_dir_all(&harness_dir);
    }

    #[test]
    fn a_project_scoped_settings_fragment_never_touches_the_harness_settings() {
        // The default: even when the daemon supplies `_harness_settings`
        // (it always resolves both paths), a `settings` fragment left at
        // its default `Project` scope must not write there.
        let dir = scratch_dir("project-scope");
        let ws = workspace_dir("project-scope-ws");
        let harness_dir = workspace_dir("project-scope-harness");
        let harness_settings = harness_dir.join("settings.json");

        let ctx = PluginCtx::new(&dir, &noop_emit);
        let mut plugin = ProfilesPlugin;
        plugin
            .handle(&ctx, req("profile.create", json!({"name": "x"})))
            .unwrap();
        plugin
            .handle(
                &ctx,
                req(
                    "profile.write_fragment",
                    json!({
                        "name": "x",
                        "fragment": "settings",
                        "content": "{\"theme\": \"dark\"}"
                    }),
                ),
            )
            .unwrap();

        let resp = ok_value(plugin.handle(
            &ctx,
            req(
                "profile.apply",
                json!({
                    "name": "x",
                    "_workspace": ws.to_string_lossy(),
                    "_harness_settings": harness_settings.to_string_lossy(),
                }),
            ),
        ));

        let changes = resp["changes"].as_array().unwrap();
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert!(!harness_settings.exists());

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&ws);
        let _ = fs::remove_dir_all(&harness_dir);
    }

    // ------------------------------------ set_remote / push / pull

    /// A fresh bare repository standing in for the private remote a
    /// profile syncs through — a real `git` transport (a filesystem
    /// path), not a fake.
    fn bare_remote(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "willie-profiles-plugin-remote-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // `-b main` matches every profile repo's own default branch
        // (see `git::init`): a bare repo's `HEAD` otherwise follows
        // whatever `init.defaultBranch` this machine's git config
        // carries, and a clone's checkout follows that — if it is not
        // `main`, a clone sees an empty working tree even though `main`
        // itself has every commit.
        git::run(&dir, &["init", "-q", "--bare", "-b", "main"]).unwrap();
        dir
    }

    /// `git clone <remote> <dest>` plus a fixed local identity — how a
    /// profile first arrives on the user's other machine (a manual
    /// clone, outside Willie); `profile.pull` carries it from there.
    fn clone_profile(remote: &Path, dest: &Path) {
        let output = std::process::Command::new("git")
            .arg("clone")
            .arg("-q")
            .arg(remote)
            .arg(dest)
            .output()
            .unwrap();
        assert!(output.status.success());
        git::run(dest, &["config", "user.name", "Willie"]).unwrap();
        git::run(dest, &["config", "user.email", "willie@localhost"]).unwrap();
    }

    #[test]
    fn set_remote_then_push_publishes_and_a_fresh_clone_pulls_the_update() {
        let dir_a = scratch_dir("sync-a");
        let dir_b = scratch_dir("sync-b");
        let remote = bare_remote("sync");
        let ctx_a = PluginCtx::new(&dir_a, &noop_emit);
        let mut plugin = ProfilesPlugin;

        plugin
            .handle(&ctx_a, req("profile.create", json!({"name": "x"})))
            .unwrap();
        plugin
            .handle(
                &ctx_a,
                req(
                    "profile.set_remote",
                    json!({"name": "x", "url": remote.to_string_lossy()}),
                ),
            )
            .unwrap();
        plugin
            .handle(&ctx_a, req("profile.push", json!({"name": "x"})))
            .unwrap();

        // A second machine's profile store: a plain `git clone` of the
        // remote, the way it would arrive there for the first time.
        clone_profile(&remote, &dir_b.join("x"));
        assert!(dir_b.join("x/profile.toml").is_file());

        // A later edit on the first machine is pushed...
        plugin
            .handle(
                &ctx_a,
                req(
                    "profile.write_fragment",
                    json!({
                        "name": "x",
                        "fragment": "instructions",
                        "content": "from a"
                    }),
                ),
            )
            .unwrap();
        plugin
            .handle(&ctx_a, req("profile.push", json!({"name": "x"})))
            .unwrap();

        // ...and `profile.pull` on the second machine fast-forwards to it.
        let ctx_b = PluginCtx::new(&dir_b, &noop_emit);
        plugin
            .handle(&ctx_b, req("profile.pull", json!({"name": "x"})))
            .unwrap();
        assert_eq!(
            fs::read_to_string(dir_b.join("x/CLAUDE.md")).unwrap(),
            "from a"
        );

        let _ = fs::remove_dir_all(&dir_a);
        let _ = fs::remove_dir_all(&dir_b);
        let _ = fs::remove_dir_all(&remote);
    }

    #[test]
    fn pull_on_a_divergent_history_is_profile_sync_conflict() {
        let dir_a = scratch_dir("conflict-a");
        let dir_b = scratch_dir("conflict-b");
        let remote = bare_remote("conflict");
        let ctx_a = PluginCtx::new(&dir_a, &noop_emit);
        let ctx_b = PluginCtx::new(&dir_b, &noop_emit);
        let mut plugin = ProfilesPlugin;

        plugin
            .handle(&ctx_a, req("profile.create", json!({"name": "x"})))
            .unwrap();
        plugin
            .handle(
                &ctx_a,
                req(
                    "profile.set_remote",
                    json!({"name": "x", "url": remote.to_string_lossy()}),
                ),
            )
            .unwrap();
        plugin
            .handle(&ctx_a, req("profile.push", json!({"name": "x"})))
            .unwrap();
        clone_profile(&remote, &dir_b.join("x"));

        // `b` edits and commits locally, without pushing.
        plugin
            .handle(
                &ctx_b,
                req(
                    "profile.write_fragment",
                    json!({
                        "name": "x",
                        "fragment": "instructions",
                        "content": "from b"
                    }),
                ),
            )
            .unwrap();

        // `a` edits, commits, and pushes ahead: the two histories now
        // diverge.
        plugin
            .handle(
                &ctx_a,
                req(
                    "profile.write_fragment",
                    json!({
                        "name": "x",
                        "fragment": "settings",
                        "content": "{\"a\": 1}"
                    }),
                ),
            )
            .unwrap();
        plugin
            .handle(&ctx_a, req("profile.push", json!({"name": "x"})))
            .unwrap();

        let err = plugin
            .handle(&ctx_b, req("profile.pull", json!({"name": "x"})))
            .unwrap_err();
        assert_eq!(err.code(), "profile_sync_conflict");

        let _ = fs::remove_dir_all(&dir_a);
        let _ = fs::remove_dir_all(&dir_b);
        let _ = fs::remove_dir_all(&remote);
    }

    #[test]
    fn an_unknown_profile_is_refused_on_set_remote_push_and_pull() {
        let dir = scratch_dir("unknown-sync");
        let ctx = PluginCtx::new(&dir, &noop_emit);
        let mut plugin = ProfilesPlugin;

        let err = plugin
            .handle(
                &ctx,
                req(
                    "profile.set_remote",
                    json!({"name": "ghost", "url": "https://example.invalid/x.git"}),
                ),
            )
            .unwrap_err();
        assert_eq!(err.code(), "profile_not_found");

        let err = plugin
            .handle(&ctx, req("profile.push", json!({"name": "ghost"})))
            .unwrap_err();
        assert_eq!(err.code(), "profile_not_found");

        let err = plugin
            .handle(&ctx, req("profile.pull", json!({"name": "ghost"})))
            .unwrap_err();
        assert_eq!(err.code(), "profile_not_found");

        let _ = fs::remove_dir_all(&dir);
    }
}
