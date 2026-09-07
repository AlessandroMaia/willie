//! `profile.toml`'s shape and the fragment-name grammar
//! (`settings|instructions|mcp|rules/<f>|hooks/<f>`), kept as typed
//! records rather than raw TOML/string values so `handle`'s four methods
//! agree on one parse.

use serde::{Deserialize, Serialize};

/// Which fragments a profile carries and, for the two file families,
/// which files. A boolean fragment (`settings`, `instructions`, `mcp`) is
/// active once turned on; a `rules`/`hooks` fragment is active once its
/// file name is in the list — the list itself is the on/off switch,
/// there is no separate flag to fall out of sync with it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Fragments {
    #[serde(default)]
    pub settings: bool,
    /// Where the `settings` fragment applies once active: the project's
    /// own workspace (`Project`, the default — applying never surprises
    /// every other project's sessions), or also the harness state every
    /// session reads (`Global`). Read only when `settings` is active.
    #[serde(default)]
    pub settings_scope: SettingsScope,
    #[serde(default)]
    pub instructions: bool,
    #[serde(default)]
    pub rules: Vec<String>,
    #[serde(default)]
    pub hooks: Vec<String>,
    #[serde(default)]
    pub mcp: bool,
}

/// Where a `settings` fragment applies (`profile.toml`'s
/// `fragments.settings_scope`). Kept as its own field rather than folded
/// into the `settings` boolean: "on" and "instead of vs. also the harness
/// state" are two different questions, and a plain flag would leave that
/// ambiguous. Defaults to `Project` — applying a profile never changes
/// every project's sessions as a surprise; a person opts into `Global`
/// explicitly.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum SettingsScope {
    #[default]
    Project,
    Global,
}

impl Fragments {
    /// The active fragment names, spelled the way `read_fragment`/
    /// `write_fragment` accept them, so a `profile.list` row lines up
    /// with what can be opened.
    #[must_use]
    pub fn active_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        if self.settings {
            names.push("settings".to_owned());
        }
        if self.instructions {
            names.push("instructions".to_owned());
        }
        if self.mcp {
            names.push("mcp".to_owned());
        }
        names.extend(self.rules.iter().map(|f| format!("rules/{f}")));
        names.extend(self.hooks.iter().map(|f| format!("hooks/{f}")));
        names
    }
}

/// `<store_dir>/<name>/profile.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub fragments: Fragments,
}

impl Profile {
    /// A freshly created profile: named, nothing active yet.
    #[must_use]
    pub fn scaffold(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            fragments: Fragments::default(),
        }
    }
}

/// `profile.list`'s row: enough to render a picker without reading every
/// fragment file's body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileSummary {
    pub name: String,
    pub fragments_active: Vec<String>,
}

impl From<&Profile> for ProfileSummary {
    fn from(profile: &Profile) -> Self {
        Self {
            name: profile.name.clone(),
            fragments_active: profile.fragments.active_names(),
        }
    }
}

/// A parsed `fragment` parameter: `settings`, `instructions`, `mcp`, or a
/// specific `rules/<f>` / `hooks/<f>` file. The relative path under a
/// profile directory each names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fragment {
    Settings,
    Instructions,
    Mcp,
    Rule(String),
    Hook(String),
}

impl Fragment {
    /// Parses the wire spelling. `None` for anything the grammar does not
    /// recognise, including a `rules/`/`hooks/` name that is empty, is
    /// `.`/`..`, or carries its own path separator (a nested path would
    /// escape the `rules/`/`hooks/` directory it is scoped to).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "settings" => Some(Self::Settings),
            "instructions" => Some(Self::Instructions),
            "mcp" => Some(Self::Mcp),
            _ => {
                if let Some(f) = s.strip_prefix("rules/") {
                    is_safe_file_name(f).then(|| Self::Rule(f.to_owned()))
                } else if let Some(f) = s.strip_prefix("hooks/") {
                    is_safe_file_name(f).then(|| Self::Hook(f.to_owned()))
                } else {
                    None
                }
            }
        }
    }

    /// The file this fragment is stored as, relative to a profile's
    /// directory.
    #[must_use]
    pub fn relative_path(&self) -> String {
        match self {
            Self::Settings => "settings.json".to_owned(),
            Self::Instructions => "CLAUDE.md".to_owned(),
            Self::Mcp => "mcp.json".to_owned(),
            Self::Rule(f) => format!("rules/{f}"),
            Self::Hook(f) => format!("hooks/{f}"),
        }
    }

    /// Marks this fragment active in `fragments` — the effect a
    /// successful `write_fragment` has on `profile.toml`: a boolean
    /// fragment is turned on, a file fragment's name is added to its
    /// family's list (turning it off again is done by editing
    /// `profile.toml` directly, per the design).
    pub fn mark_active(&self, fragments: &mut Fragments) {
        match self {
            Self::Settings => fragments.settings = true,
            Self::Instructions => fragments.instructions = true,
            Self::Mcp => fragments.mcp = true,
            Self::Rule(f) => push_once(&mut fragments.rules, f),
            Self::Hook(f) => push_once(&mut fragments.hooks, f),
        }
    }
}

fn push_once(list: &mut Vec<String>, item: &str) {
    if !list.iter().any(|existing| existing == item) {
        list.push(item.to_owned());
    }
}

/// A safe single-path-segment file name: non-empty, no path separator,
/// not a `.`/`..` component, and not a Windows drive-letter prefix
/// (`C:foo`, `a:bar`). The drive-letter check matters even though `willied`
/// only ever runs on Linux: this crate carries no `cfg(target_os =
/// "linux")` of its own, so `cargo test --workspace` on a Windows
/// development machine builds and runs it under real Windows path
/// semantics, where `Path::join` treats a drive-prefixed argument as an
/// absolute replacement of the base rather than a child of it.
fn is_safe_file_name(f: &str) -> bool {
    !f.is_empty()
        && !f.contains('/')
        && !f.contains('\\')
        && f != "."
        && f != ".."
        && !starts_with_drive_letter(f)
}

/// Whether `s` opens with a Windows drive-letter pattern: an ASCII letter
/// immediately followed by `:`.
fn starts_with_drive_letter(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.next() == Some(':')
}

/// Whether `name` is a safe profile directory name: non-empty, no path
/// separator, and not a `.`/`..` component — the same rule as a fragment's
/// file name, since both become a path segment under `store_dir`.
#[must_use]
pub fn is_valid_profile_name(name: &str) -> bool {
    is_safe_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scaffolded_profile_has_no_active_fragments() {
        let profile = Profile::scaffold("funcef-auth");
        assert_eq!(profile.name, "funcef-auth");
        assert!(profile.fragments.active_names().is_empty());
    }

    #[test]
    fn a_scaffolded_profile_defaults_its_settings_scope_to_project() {
        let profile = Profile::scaffold("funcef-auth");
        assert_eq!(profile.fragments.settings_scope, SettingsScope::Project);
    }

    #[test]
    fn a_profile_toml_with_no_settings_scope_defaults_to_project() {
        let back: Profile =
            toml::from_str("name = \"x\"\n[fragments]\nsettings = true\n")
                .unwrap();
        assert_eq!(back.fragments.settings_scope, SettingsScope::Project);
    }

    #[test]
    fn settings_scope_round_trips_through_toml() {
        let mut profile = Profile::scaffold("x");
        profile.fragments.settings = true;
        profile.fragments.settings_scope = SettingsScope::Global;

        let text = toml::to_string_pretty(&profile).unwrap();
        let back: Profile = toml::from_str(&text).unwrap();

        assert_eq!(back.fragments.settings_scope, SettingsScope::Global);
        assert!(text.contains("settings_scope = \"global\""), "{text}");
    }

    #[test]
    fn profile_toml_round_trips_through_toml() {
        let profile = Profile {
            name: "funcef-auth".to_owned(),
            fragments: Fragments {
                settings: true,
                settings_scope: SettingsScope::Project,
                instructions: true,
                rules: vec!["no-force-push.md".to_owned()],
                hooks: vec![],
                mcp: true,
            },
        };

        let text = toml::to_string_pretty(&profile).unwrap();
        let back: Profile = toml::from_str(&text).unwrap();

        assert_eq!(back, profile);
    }

    #[test]
    fn a_profile_toml_with_no_fragments_table_defaults_to_empty() {
        let back: Profile = toml::from_str("name = \"bare\"\n").unwrap();
        assert_eq!(back, Profile::scaffold("bare"));
    }

    #[test]
    fn active_names_lines_up_with_the_fragment_grammar() {
        let fragments = Fragments {
            settings: true,
            settings_scope: SettingsScope::Project,
            instructions: false,
            rules: vec!["a.md".to_owned(), "b.md".to_owned()],
            hooks: vec!["pre-commit".to_owned()],
            mcp: true,
        };
        assert_eq!(
            fragments.active_names(),
            vec![
                "settings".to_owned(),
                "mcp".to_owned(),
                "rules/a.md".to_owned(),
                "rules/b.md".to_owned(),
                "hooks/pre-commit".to_owned(),
            ]
        );
    }

    #[test]
    fn summary_from_profile_carries_the_active_names() {
        let mut profile = Profile::scaffold("x");
        profile.fragments.mcp = true;
        let summary = ProfileSummary::from(&profile);
        assert_eq!(summary.name, "x");
        assert_eq!(summary.fragments_active, vec!["mcp".to_owned()]);
    }

    #[test]
    fn fragment_parses_the_fixed_names() {
        assert_eq!(Fragment::parse("settings"), Some(Fragment::Settings));
        assert_eq!(
            Fragment::parse("instructions"),
            Some(Fragment::Instructions)
        );
        assert_eq!(Fragment::parse("mcp"), Some(Fragment::Mcp));
    }

    #[test]
    fn fragment_parses_a_rule_or_hook_file() {
        assert_eq!(
            Fragment::parse("rules/no-force-push.md"),
            Some(Fragment::Rule("no-force-push.md".to_owned()))
        );
        assert_eq!(
            Fragment::parse("hooks/pre-commit"),
            Some(Fragment::Hook("pre-commit".to_owned()))
        );
    }

    #[test]
    fn fragment_rejects_an_unknown_name() {
        assert_eq!(Fragment::parse("nope"), None);
        assert_eq!(Fragment::parse(""), None);
    }

    #[test]
    fn fragment_rejects_a_rule_or_hook_name_that_escapes_its_directory() {
        assert_eq!(Fragment::parse("rules/"), None);
        assert_eq!(Fragment::parse("rules/.."), None);
        assert_eq!(Fragment::parse("rules/../../etc/passwd"), None);
        assert_eq!(Fragment::parse("hooks/sub/dir"), None);
    }

    #[test]
    fn relative_path_maps_each_variant_to_its_file() {
        assert_eq!(Fragment::Settings.relative_path(), "settings.json");
        assert_eq!(Fragment::Instructions.relative_path(), "CLAUDE.md");
        assert_eq!(Fragment::Mcp.relative_path(), "mcp.json");
        assert_eq!(
            Fragment::Rule("a.md".to_owned()).relative_path(),
            "rules/a.md"
        );
        assert_eq!(
            Fragment::Hook("pre-commit".to_owned()).relative_path(),
            "hooks/pre-commit"
        );
    }

    #[test]
    fn mark_active_turns_on_a_boolean_fragment() {
        let mut fragments = Fragments::default();
        Fragment::Mcp.mark_active(&mut fragments);
        assert!(fragments.mcp);
    }

    #[test]
    fn mark_active_adds_a_rule_file_once() {
        let mut fragments = Fragments::default();
        let rule = Fragment::Rule("a.md".to_owned());
        rule.mark_active(&mut fragments);
        rule.mark_active(&mut fragments);
        assert_eq!(fragments.rules, vec!["a.md".to_owned()]);
    }

    #[test]
    fn profile_names_reject_path_separators_and_dot_dot() {
        assert!(is_valid_profile_name("funcef-auth"));
        assert!(!is_valid_profile_name(""));
        assert!(!is_valid_profile_name(".."));
        assert!(!is_valid_profile_name("."));
        assert!(!is_valid_profile_name("a/b"));
        assert!(!is_valid_profile_name("a\\b"));
        assert!(!is_valid_profile_name("../escape"));
    }

    #[test]
    fn profile_names_reject_a_windows_drive_letter_prefix() {
        // `Path::join` on Windows treats a drive-prefixed argument as an
        // absolute replacement of the base, not a child of it (`C:foo` and
        // even the unusual-but-valid `a:bar` both qualify) — refused
        // before any path is ever built from the name.
        assert!(!is_valid_profile_name("C:foo"));
        assert!(!is_valid_profile_name("c:foo"));
        assert!(!is_valid_profile_name("a:bar"));
        assert!(!is_valid_profile_name("Z:\\escape"));
        // A bare colon with nothing recognisable as a drive letter first
        // is not this pattern, but still fails on its own merits
        // elsewhere; not a false positive to worry about here.
        assert!(is_valid_profile_name("safe-name"));
    }
}
