//! Configuration profiles applied per project.
//!
//! A profile is a versioned directory of fragments (settings, instructions,
//! rules, hooks, MCP servers). Applying one writes into the project and
//! into the harness state while preserving the existing files' format.
//!
//! Phase 1 (this crate's `handle`) only creates a profile and lets its
//! fragments be read and written, each edit its own git commit inside the
//! profile's own repository at `<store_dir>/<name>/`. The format-preserving
//! merge into a project (`profile.check`/`profile.apply`) and the minimal
//! sync (`profile.push`/`pull`) are later phases of this slice.

mod git;
mod model;

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use serde_json::{Value, json};
use willie_plugin_api::{
    Plugin, PluginCtx, PluginError, PluginManifest, PluginRequest,
    PluginResponse, Scope,
};

use model::{Fragment, Profile, ProfileSummary, is_valid_profile_name};

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
    if !is_valid_profile_name(&name) {
        return Err(invalid_name(&name));
    }

    let dir = profile_dir(ctx, &name);
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

// --------------------------------------------------------------- helpers

/// `<store_dir>/<name>/`, not necessarily existing yet.
fn profile_dir(ctx: &PluginCtx<'_>, name: &str) -> PathBuf {
    ctx.store_dir().join(name)
}

/// Resolves `name` to an existing profile's directory, refusing an
/// invalid name or one with no `profile.toml` as `profile_not_found` — the
/// two are indistinguishable to the caller (a name that could never be
/// valid never corresponds to a real profile either).
fn existing_profile_dir(
    ctx: &PluginCtx<'_>,
    name: &str,
) -> Result<PathBuf, PluginError> {
    if !is_valid_profile_name(name) {
        return Err(not_found(name));
    }
    let dir = profile_dir(ctx, name);
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
    fn a_path_traversal_name_is_not_found_not_a_fault() {
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
        assert_eq!(err.code(), "profile_not_found");
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
}
