//! The project domain type and its pure helpers. No I/O.

use serde::{Deserialize, Serialize};

use crate::id::ProjectId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProjectState {
    Preparing,
    Ready,
    Failed {
        code: String,
        message: String,
        remediation: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub slug: String,
    /// The Windows checkout path as registered, e.g. `C:\github\x`.
    pub source: String,
    /// The ext4 clone, e.g. `/home/willie/projects/x`.
    pub workspace: String,
    pub branch: String,
    pub state: ProjectState,
    /// Recomputed by the daemon per snapshot, never trusted from disk.
    #[serde(default = "yes")]
    pub source_present: bool,
    pub created_at: String,
}

fn yes() -> bool {
    true
}

/// Kebab-case ASCII slug, made unique against `taken` by a numeric suffix.
#[must_use]
pub fn slug_for(folder_name: &str, taken: &[String]) -> String {
    let base = kebab_ascii(folder_name);
    let base = if base.is_empty() {
        "project".to_owned()
    } else {
        base
    };
    if !taken.iter().any(|t| t == &base) {
        return base;
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{base}-{n}");
        if !taken.iter().any(|t| t == &candidate) {
            return candidate;
        }
        n += 1;
    }
}

fn kebab_ascii(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in s.chars() {
        let mapped = fold_ascii(ch);
        if mapped.is_ascii_alphanumeric() {
            out.push(mapped.to_ascii_lowercase());
            dash = false;
        } else if !out.is_empty() && !dash {
            out.push('-');
            dash = true;
        }
    }
    out.trim_matches('-').to_owned()
}

/// Best-effort fold of common Latin accents so a slug stays ASCII.
fn fold_ascii(ch: char) -> char {
    match ch {
        'á' | 'à' | 'â' | 'ã' | 'ä' | 'Á' | 'À' | 'Â' | 'Ã' | 'Ä' => {
            'a'
        }
        'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' | 'Í' | 'Ì' | 'Î' | 'Ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' | 'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' => {
            'o'
        }
        'ú' | 'ù' | 'û' | 'ü' | 'Ú' | 'Ù' | 'Û' | 'Ü' => 'u',
        'ç' | 'Ç' => 'c',
        'ñ' | 'Ñ' => 'n',
        other => other,
    }
}

/// Identity of a Windows checkout: lower-cased, forward slashes, no
/// trailing separator, so the same folder is never registered twice.
#[must_use]
pub fn source_key(windows_path: &str) -> String {
    windows_path
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_is_kebab_ascii_and_avoids_collisions() {
        assert_eq!(slug_for("Web Template", &[]), "web-template");
        assert_eq!(slug_for("café_app", &[]), "cafe-app");
        assert_eq!(slug_for("willie", &["willie".to_owned()]), "willie-2");
        assert_eq!(
            slug_for("willie", &["willie".to_owned(), "willie-2".to_owned()]),
            "willie-3"
        );
    }

    #[test]
    fn source_key_is_case_and_separator_insensitive() {
        assert_eq!(
            source_key(r"C:\GitHub\Willie\"),
            source_key(r"c:/github/willie")
        );
    }

    #[test]
    fn project_round_trips_through_toml() {
        let p = Project {
            id: ProjectId::new(),
            name: "Willie".into(),
            slug: "willie".into(),
            source: r"C:\github\pessoal\willie".into(),
            workspace: "/home/willie/projects/willie".into(),
            branch: "main".into(),
            state: ProjectState::Ready,
            source_present: true,
            created_at: "2026-08-26T00:00:00Z".into(),
        };
        let text = toml::to_string(&p).unwrap();
        let back: Project = toml::from_str(&text).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn source_present_defaults_true_when_absent_from_toml() {
        let text = "\
id = \"proj_00000000000000000000000000\"
name = \"x\"
slug = \"x\"
source = \"C:/x\"
workspace = \"/home/willie/projects/x\"
branch = \"main\"
created_at = \"t\"
[state]
state = \"ready\"
";
        let p: Project = toml::from_str(text).unwrap();
        assert!(p.source_present);
    }
}
