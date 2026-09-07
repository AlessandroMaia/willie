//! The format-preserving merge (design's Phase 2), pure over its inputs.
//!
//! Every function here takes and returns plain strings or in-memory
//! records — no filesystem, no git. `profile.check`/`profile.apply`
//! (task 6) read a profile's fragment files and a project's target
//! files, hand their content in as [`Targets`], and write back whatever
//! [`plan_changes`] returns; the write and the backup are task 6's job,
//! not this module's.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{Fragments, Profile};

/// A merge or plan failure, carrying a stable code for the apply step
/// (task 6) to turn into a `PluginError::coded`. Kept distinct from
/// `PluginError` itself: this module has no business knowing about the
/// plugin host's error shape, only about naming what went wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyError {
    pub code: &'static str,
    pub message: String,
}

impl ApplyError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// One fragment's resolved inputs against a single destination: the
/// fragment's own content (read from the profile directory) and the
/// destination's current content, `None` when it does not exist yet.
/// When two fragments target the same destination file (`settings` and
/// `mcp` both merge into `.claude/settings.json`), the caller repeats
/// the same `existing` content in both entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FragmentContent {
    pub fragment: String,
    pub existing: Option<String>,
}

/// The resolved inputs `plan_changes` needs for every fragment the
/// profile marks active — populated by task 6 from the profile
/// directory and the project's (or the harness state's) current files,
/// so this stays pure. A field left `None`/empty for a fragment the
/// profile marks active is a caller error, refused rather than
/// silently skipped (see `missing_target`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Targets {
    /// `.claude/settings.json`'s inputs, present when the `settings`
    /// fragment is active.
    pub settings: Option<FragmentContent>,
    /// The `mcp` fragment's inputs; also targets
    /// `.claude/settings.json`, merged under its `mcpServers` key.
    pub mcp: Option<FragmentContent>,
    /// The project's `CLAUDE.md` inputs, present when the
    /// `instructions` fragment is active.
    pub instructions: Option<FragmentContent>,
    /// Each active rule file's inputs, keyed by file name (matching
    /// `Fragments::rules`).
    pub rules: HashMap<String, FragmentContent>,
    /// Each active hook file's inputs, keyed by file name (matching
    /// `Fragments::hooks`).
    pub hooks: HashMap<String, FragmentContent>,
}

/// What applying one change does to its destination file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Create,
    Merge,
    Overwrite,
}

/// One destination file `profile.check`/`profile.apply` would touch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    /// Relative to the project workspace (or the harness state dir, for
    /// a `settings` fragment marked `scope = "global"` — task 6's
    /// concern, not this module's).
    pub path: String,
    pub kind: ChangeKind,
    /// The full content the destination would have after the change.
    pub after: String,
}

const MARKER_BEGIN: &str = "<!-- willie:begin -->";
const MARKER_END: &str = "<!-- willie:end -->";

/// Deep-merges `fragment` onto `target`, both parsed as ordered JSON
/// maps (`serde_json` built with `preserve_order`): the target's
/// existing keys keep their position, a key the fragment also carries
/// is merged in place (recursively, for a nested object) or overwritten
/// (for anything else), and a key only the fragment has is appended
/// after the target's own. Re-serialises with a 2-space indent and a
/// trailing newline. An invalid target or fragment is
/// `profile_fragment_invalid`.
pub fn merge_json(target: &str, fragment: &str) -> Result<String, ApplyError> {
    merge_value(target, fragment, None)
}

/// Replaces only the region between `<!-- willie:begin -->` and
/// `<!-- willie:end -->` with `fragment`; the person's own prose
/// outside the markers is untouched, byte for byte. Three shapes of
/// `target`:
/// - both markers present, end after begin: replaces the span between
///   them (inclusive of the markers themselves).
/// - no begin marker anywhere (this also covers a lone `willie:end`
///   with no matching begin — there is no region to grow, so it reads
///   the same as "absent"): appends the marker block at the end.
/// - a begin marker with **no** matching end after it: refused as
///   `profile_markers_malformed` rather than silently appending a
///   second block after the orphaned begin, which would never
///   converge on repeated applies (each call would append yet
///   another block).
pub fn merge_markdown(
    target: &str,
    fragment: &str,
) -> Result<String, ApplyError> {
    let block = marker_block(fragment);
    match marker_span(target)? {
        Some((start, end)) => {
            let mut result = String::with_capacity(target.len() + block.len());
            result.push_str(&target[..start]);
            result.push_str(&block);
            result.push_str(&target[end..]);
            Ok(result)
        }
        None => Ok(append_block(target, &block)),
    }
}

/// Lists a `Change` for every fragment `profile` marks active, computed
/// purely from `targets` — no filesystem access. A missing target is a
/// `Create`; an existing one is a `Merge` (JSON/Markdown fragments) or
/// an `Overwrite` (a copied rule/hook file). An active fragment with no
/// matching entry in `targets` is `profile_fragment_missing` — task 6's
/// contract is to populate every active fragment's inputs before
/// calling this. A fragment's content that fails to parse (JSON) or
/// carries an unterminated marker (Markdown) is `profile_fragment_invalid`
/// / `profile_markers_malformed` instead — a content fault, not a wiring
/// one.
pub fn plan_changes(
    profile: &Profile,
    targets: &Targets,
) -> Result<Vec<Change>, ApplyError> {
    let mut changes = Vec::new();

    if let Some(change) = plan_settings(&profile.fragments, targets)? {
        changes.push(change);
    }
    if profile.fragments.instructions {
        changes.push(plan_instructions(targets)?);
    }
    changes.extend(plan_files(
        &profile.fragments.rules,
        &targets.rules,
        ".claude/rules",
    )?);
    changes.extend(plan_files(
        &profile.fragments.hooks,
        &targets.hooks,
        ".claude/hooks",
    )?);

    Ok(changes)
}

// --------------------------------------------------------------- JSON

fn merge_value(
    target: &str,
    fragment: &str,
    under: Option<&str>,
) -> Result<String, ApplyError> {
    let mut target_value = parse_json(target, "target")?;
    let fragment_value = parse_json(fragment, "fragment")?;

    match under {
        None => deep_merge(&mut target_value, fragment_value),
        Some(key) => {
            let Value::Object(target_map) = &mut target_value else {
                return Err(ApplyError::new(
                    "profile_fragment_invalid",
                    "the target is not a JSON object",
                ));
            };
            let slot = target_map
                .entry(key.to_owned())
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            deep_merge(slot, fragment_value);
        }
    }

    serialize_pretty(&target_value)
}

/// Merges `fragment` under `target`'s `key`, creating an empty object
/// there first if it is absent — the `mcp` fragment's own merge, one
/// level under `.claude/settings.json`'s `mcpServers`.
fn merge_under(
    target: &str,
    key: &str,
    fragment: &str,
) -> Result<String, ApplyError> {
    merge_value(target, fragment, Some(key))
}

fn parse_json(text: &str, which: &str) -> Result<Value, ApplyError> {
    serde_json::from_str(text).map_err(|e| {
        ApplyError::new(
            "profile_fragment_invalid",
            format!("the {which} is not valid JSON: {e}"),
        )
    })
}

fn serialize_pretty(value: &Value) -> Result<String, ApplyError> {
    let mut out = serde_json::to_string_pretty(value).map_err(|e| {
        ApplyError::new(
            "profile_fragment_invalid",
            format!("cannot serialise the merged JSON: {e}"),
        )
    })?;
    out.push('\n');
    Ok(out)
}

/// Merges `fragment` into `target` in place. An object key both sides
/// carry is merged recursively (or overwritten, if the fragment's value
/// there is not itself an object); a key only the fragment has is
/// inserted, landing after the target's existing keys because
/// `serde_json`'s `preserve_order` map is insertion-ordered. Anything
/// that is not an object on the fragment's side replaces the target
/// wholesale, matching plain JSON-merge-patch semantics for scalars and
/// arrays.
fn deep_merge(target: &mut Value, fragment: Value) {
    match fragment {
        Value::Object(fragment_map) => {
            if let Value::Object(target_map) = target {
                for (key, value) in fragment_map {
                    match target_map.get_mut(&key) {
                        Some(existing) => deep_merge(existing, value),
                        None => {
                            target_map.insert(key, value);
                        }
                    }
                }
            } else {
                *target = Value::Object(fragment_map);
            }
        }
        other => *target = other,
    }
}

// ----------------------------------------------------------- Markdown

fn marker_block(fragment: &str) -> String {
    format!(
        "{MARKER_BEGIN}\n{}\n{MARKER_END}",
        fragment.trim_matches('\n')
    )
}

/// The byte range `[start, end)` a replacement should span: from the
/// start of `<!-- willie:begin -->` to just past the end of
/// `<!-- willie:end -->`. `Ok(None)` when there is no begin marker at
/// all (a lone end marker with nothing before it reads the same way —
/// there is no region to grow). `Err(profile_markers_malformed)` when a
/// begin marker is found but no end marker follows it: an orphaned
/// begin is refused rather than silently growing a second block on
/// every subsequent merge.
fn marker_span(target: &str) -> Result<Option<(usize, usize)>, ApplyError> {
    let Some(start) = target.find(MARKER_BEGIN) else {
        return Ok(None);
    };
    let after_begin = start + MARKER_BEGIN.len();
    let Some(end_rel) = target[after_begin..].find(MARKER_END) else {
        return Err(ApplyError::new(
            "profile_markers_malformed",
            "the willie-managed block is missing its closing \
             <!-- willie:end --> marker; fix or remove the stray \
             <!-- willie:begin --> marker, then apply again",
        ));
    };
    let end = after_begin + end_rel + MARKER_END.len();
    Ok(Some((start, end)))
}

/// Appends `block` at the end of `target`, separated from any existing
/// prose by a blank line, and leaves every existing byte of `target`
/// untouched ahead of it.
fn append_block(target: &str, block: &str) -> String {
    if target.is_empty() {
        return format!("{block}\n");
    }
    let mut result = target.to_owned();
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result.push('\n');
    result.push_str(block);
    result.push('\n');
    result
}

// --------------------------------------------------------------- plan

/// A caller-wiring fault, not a content fault: `plan_changes` was asked
/// to apply a fragment the profile marks active, but `targets` carries
/// no entry for it. Distinct from `profile_fragment_invalid` (a
/// fragment whose *content* failed to parse) so the two failure classes
/// are distinguishable by code, not only by message.
fn missing_target(fragment: &str) -> ApplyError {
    ApplyError::new(
        "profile_fragment_missing",
        format!(
            "the `{fragment}` fragment is active but no target content \
             was supplied"
        ),
    )
}

fn plan_settings(
    fragments: &Fragments,
    targets: &Targets,
) -> Result<Option<Change>, ApplyError> {
    if !fragments.settings && !fragments.mcp {
        return Ok(None);
    }

    let existing = targets
        .settings
        .as_ref()
        .and_then(|f| f.existing.clone())
        .or_else(|| targets.mcp.as_ref().and_then(|f| f.existing.clone()));
    let mut current = existing.clone().unwrap_or_else(|| "{}".to_owned());

    if fragments.settings {
        let content = targets
            .settings
            .as_ref()
            .ok_or_else(|| missing_target("settings"))?;
        current = merge_json(&current, &content.fragment)?;
    }
    if fragments.mcp {
        let content =
            targets.mcp.as_ref().ok_or_else(|| missing_target("mcp"))?;
        current = merge_under(&current, "mcpServers", &content.fragment)?;
    }

    Ok(Some(Change {
        path: ".claude/settings.json".to_owned(),
        kind: if existing.is_some() {
            ChangeKind::Merge
        } else {
            ChangeKind::Create
        },
        after: current,
    }))
}

fn plan_instructions(targets: &Targets) -> Result<Change, ApplyError> {
    let content = targets
        .instructions
        .as_ref()
        .ok_or_else(|| missing_target("instructions"))?;
    let existing = content.existing.clone().unwrap_or_default();
    let after = merge_markdown(&existing, &content.fragment)?;
    Ok(Change {
        path: "CLAUDE.md".to_owned(),
        kind: if content.existing.is_some() {
            ChangeKind::Merge
        } else {
            ChangeKind::Create
        },
        after,
    })
}

fn plan_files(
    names: &[String],
    inputs: &HashMap<String, FragmentContent>,
    dest_dir: &str,
) -> Result<Vec<Change>, ApplyError> {
    let mut changes = Vec::new();
    for name in names {
        let content = inputs.get(name).ok_or_else(|| missing_target(name))?;
        changes.push(Change {
            path: format!("{dest_dir}/{name}"),
            kind: if content.existing.is_some() {
                ChangeKind::Overwrite
            } else {
                ChangeKind::Create
            },
            after: content.fragment.clone(),
        });
    }
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Profile;

    fn content(fragment: &str, existing: Option<&str>) -> FragmentContent {
        FragmentContent {
            fragment: fragment.to_owned(),
            existing: existing.map(str::to_owned),
        }
    }

    // ----------------------------------------------------- merge_json

    #[test]
    fn merge_json_keeps_the_targets_key_order_and_appends_new_keys() {
        let target = r#"{"b": 1, "a": 2}"#;
        let fragment = r#"{"a": 20, "c": 3}"#;

        let merged = merge_json(target, fragment).unwrap();

        // `b` first, then `a` (updated, still in its original spot),
        // then `c` appended after both — never alphabetical.
        let b_pos = merged.find("\"b\"").unwrap();
        let a_pos = merged.find("\"a\"").unwrap();
        let c_pos = merged.find("\"c\"").unwrap();
        assert!(b_pos < a_pos, "merged JSON: {merged}");
        assert!(a_pos < c_pos, "merged JSON: {merged}");

        let value: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(value["a"], 20);
        assert_eq!(value["b"], 1);
        assert_eq!(value["c"], 3);
    }

    #[test]
    fn merge_json_reserialises_with_a_two_space_indent() {
        let merged = merge_json("{}", r#"{"a": 1}"#).unwrap();
        assert_eq!(merged, "{\n  \"a\": 1\n}\n");
    }

    #[test]
    fn merge_json_merges_a_nested_object_like_mcp_servers() {
        let target = r#"{"mcpServers": {"a": {"url": "x"}}}"#;
        let fragment = r#"{"mcpServers": {"b": {"url": "y"}}}"#;

        let merged = merge_json(target, fragment).unwrap();
        let value: Value = serde_json::from_str(&merged).unwrap();

        assert_eq!(value["mcpServers"]["a"]["url"], "x");
        assert_eq!(value["mcpServers"]["b"]["url"], "y");
    }

    #[test]
    fn merge_json_refuses_an_invalid_target() {
        let err = merge_json("not json", "{}").unwrap_err();
        assert_eq!(err.code, "profile_fragment_invalid");
        assert!(err.message.contains("target"), "message: {}", err.message);
    }

    #[test]
    fn merge_json_refuses_an_invalid_fragment() {
        let err = merge_json("{}", "not json").unwrap_err();
        assert_eq!(err.code, "profile_fragment_invalid");
        assert!(err.message.contains("fragment"), "message: {}", err.message);
    }

    // ------------------------------------------------- merge_markdown

    #[test]
    fn merge_markdown_replaces_only_between_the_markers() {
        let target = "# Notes\n\nbefore\n\n<!-- willie:begin -->\nold\n<!-- willie:end -->\n\nafter\n";

        let merged = merge_markdown(target, "new").unwrap();

        assert_eq!(
            merged,
            "# Notes\n\nbefore\n\n<!-- willie:begin -->\nnew\n<!-- willie:end -->\n\nafter\n"
        );
    }

    #[test]
    fn merge_markdown_leaves_outside_prose_byte_for_byte() {
        let prose = "# Notes\r\nweird\ttabs and trailing spaces   \n\u{e9}";
        let target = format!(
            "{prose}\n<!-- willie:begin -->\nold\n<!-- willie:end -->\n"
        );

        let merged = merge_markdown(&target, "new").unwrap();

        assert!(merged.starts_with(prose), "merged: {merged:?}");
    }

    #[test]
    fn merge_markdown_inserts_the_block_at_the_end_when_absent() {
        let target = "# Notes\n\nsome prose\n";

        let merged = merge_markdown(target, "new").unwrap();

        assert_eq!(
            merged,
            "# Notes\n\nsome prose\n\n<!-- willie:begin -->\nnew\n<!-- willie:end -->\n"
        );
    }

    #[test]
    fn merge_markdown_on_an_empty_target_is_just_the_block() {
        let merged = merge_markdown("", "new").unwrap();
        assert_eq!(merged, "<!-- willie:begin -->\nnew\n<!-- willie:end -->\n");
    }

    #[test]
    fn merge_markdown_refuses_an_unterminated_begin_marker() {
        // A begin marker with no matching end: must not silently
        // append a second block after the orphan (that would never
        // converge — every subsequent apply would append yet another).
        let target =
            "# Notes\n\n<!-- willie:begin -->\norphaned, no end marker\n";

        let err = merge_markdown(target, "new").unwrap_err();

        assert_eq!(err.code, "profile_markers_malformed");
    }

    #[test]
    fn merge_markdown_treats_a_lone_end_marker_as_absent_and_appends() {
        // No begin marker at all — even though an end marker is
        // present, there is no region to grow, so this reads the same
        // as "absent": append a fresh, well-formed block.
        let target = "# Notes\n\nstray <!-- willie:end --> with no begin\n";

        let merged = merge_markdown(target, "new").unwrap();

        assert!(merged.starts_with(target));
        assert!(
            merged
                .ends_with("<!-- willie:begin -->\nnew\n<!-- willie:end -->\n")
        );
    }

    #[test]
    fn merge_markdown_converges_on_a_second_apply() {
        // Applying the same fragment twice to an already well-formed
        // target must be a no-op the second time — the idempotency the
        // apply step relies on.
        let target = "# Notes\n\nprose\n";

        let first = merge_markdown(target, "new").unwrap();
        let second = merge_markdown(&first, "new").unwrap();

        assert_eq!(first, second);
    }

    // --------------------------------------------------- plan_changes

    #[test]
    fn plan_changes_lists_a_create_for_a_missing_target() {
        let mut profile = Profile::scaffold("x");
        profile.fragments.settings = true;
        let targets = Targets {
            settings: Some(content(r#"{"a": 1}"#, None)),
            ..Targets::default()
        };

        let changes = plan_changes(&profile, &targets).unwrap();

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, ".claude/settings.json");
        assert_eq!(changes[0].kind, ChangeKind::Create);
    }

    #[test]
    fn plan_changes_lists_a_merge_for_an_existing_target() {
        let mut profile = Profile::scaffold("x");
        profile.fragments.settings = true;
        let targets = Targets {
            settings: Some(content(r#"{"a": 1}"#, Some(r#"{"b": 2}"#))),
            ..Targets::default()
        };

        let changes = plan_changes(&profile, &targets).unwrap();

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, ChangeKind::Merge);
        let value: Value = serde_json::from_str(&changes[0].after).unwrap();
        assert_eq!(value["a"], 1);
        assert_eq!(value["b"], 2);
    }

    #[test]
    fn plan_changes_combines_settings_and_mcp_into_one_change() {
        let mut profile = Profile::scaffold("x");
        profile.fragments.settings = true;
        profile.fragments.mcp = true;
        let targets = Targets {
            settings: Some(content(r#"{"theme": "dark"}"#, None)),
            mcp: Some(content(r#"{"my-server": {"url": "x"}}"#, None)),
            ..Targets::default()
        };

        let changes = plan_changes(&profile, &targets).unwrap();

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, ".claude/settings.json");
        let value: Value = serde_json::from_str(&changes[0].after).unwrap();
        assert_eq!(value["theme"], "dark");
        assert_eq!(value["mcpServers"]["my-server"]["url"], "x");
    }

    #[test]
    fn plan_changes_lists_a_merge_for_instructions_between_the_markers() {
        let mut profile = Profile::scaffold("x");
        profile.fragments.instructions = true;
        let existing =
            "# Notes\n\n<!-- willie:begin -->\nold\n<!-- willie:end -->\n";
        let targets = Targets {
            instructions: Some(content("new", Some(existing))),
            ..Targets::default()
        };

        let changes = plan_changes(&profile, &targets).unwrap();

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "CLAUDE.md");
        assert_eq!(changes[0].kind, ChangeKind::Merge);
        assert!(changes[0].after.contains("new"));
        assert!(!changes[0].after.contains("old"));
    }

    #[test]
    fn plan_changes_overwrites_an_existing_rule_file_and_creates_a_new_one() {
        let mut profile = Profile::scaffold("x");
        profile.fragments.rules = vec!["a.md".to_owned(), "b.md".to_owned()];
        let mut rules = HashMap::new();
        rules.insert("a.md".to_owned(), content("A content", Some("old A")));
        rules.insert("b.md".to_owned(), content("B content", None));
        let targets = Targets {
            rules,
            ..Targets::default()
        };

        let mut changes = plan_changes(&profile, &targets).unwrap();
        changes.sort_by(|a, b| a.path.cmp(&b.path));

        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].path, ".claude/rules/a.md");
        assert_eq!(changes[0].kind, ChangeKind::Overwrite);
        assert_eq!(changes[0].after, "A content");
        assert_eq!(changes[1].path, ".claude/rules/b.md");
        assert_eq!(changes[1].kind, ChangeKind::Create);
    }

    #[test]
    fn plan_changes_is_empty_for_a_profile_with_no_active_fragments() {
        let profile = Profile::scaffold("x");
        let changes = plan_changes(&profile, &Targets::default()).unwrap();
        assert!(changes.is_empty());
    }

    #[test]
    fn plan_changes_refuses_an_active_fragment_with_no_target_content() {
        let mut profile = Profile::scaffold("x");
        profile.fragments.settings = true;

        let err = plan_changes(&profile, &Targets::default()).unwrap_err();

        // A wiring fault (task 6 forgot to populate the target), not a
        // content fault — distinct from `profile_fragment_invalid`.
        assert_eq!(err.code, "profile_fragment_missing");
    }
}
