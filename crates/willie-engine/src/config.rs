//! `engine.toml` — the only Windows-side state besides the image. Holds
//! the machine profile and UI preferences; this slice adds project roots.

use std::{fs, io, path::Path};

use serde::{Deserialize, Serialize};
use willie_core::id::ProjectId;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineConfig {
    #[serde(default)]
    pub projects: Projects,
    #[serde(default)]
    pub ui: Ui,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Projects {
    #[serde(default)]
    pub roots: Vec<String>,
}

/// UI preferences: which system the sidebar shows selected, restored
/// across restarts. TOML has no null, so an unset preference is a
/// missing key rather than one written as `null`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ui {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_project: Option<ProjectId>,
}

impl EngineConfig {
    #[must_use]
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = toml::to_string(self)
            .map_err(|e| io::Error::other(e.to_string()))?;
        fs::write(path, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_empty_and_round_trips() {
        let dir = std::env::temp_dir()
            .join(format!("willie-cfg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("engine.toml");
        assert!(EngineConfig::load(&path).projects.roots.is_empty());
        let cfg = EngineConfig {
            projects: Projects {
                roots: vec![r"C:\github".into()],
            },
            ui: Ui::default(),
        };
        cfg.save(&path).unwrap();
        assert_eq!(EngineConfig::load(&path), cfg);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let cfg: EngineConfig =
            toml::from_str("future = true\n[projects]\nroots = []\n").unwrap();
        assert!(cfg.projects.roots.is_empty());
    }

    /// The `[ui]` table round-trips through `save`/`load`; an absent
    /// table (or an absent file) resolves to `None`, never a placeholder
    /// id, and a later save that only means to touch `[ui]` still keeps
    /// `projects.roots` intact.
    #[test]
    fn engine_config_reads_and_writes_the_ui_table() {
        let dir = std::env::temp_dir()
            .join(format!("willie-cfg-ui-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("engine.toml");

        assert!(EngineConfig::load(&path).ui.current_project.is_none());

        let mut cfg = EngineConfig {
            projects: Projects {
                roots: vec![r"C:\github".into()],
            },
            ui: Ui {
                current_project: Some(ProjectId::new()),
            },
        };
        cfg.save(&path).unwrap();
        assert_eq!(EngineConfig::load(&path), cfg);

        cfg.ui.current_project = None;
        cfg.save(&path).unwrap();
        let reloaded = EngineConfig::load(&path);
        assert_eq!(reloaded.projects.roots, vec![r"C:\github".to_owned()]);
        assert!(reloaded.ui.current_project.is_none());

        let _ = fs::remove_dir_all(&dir);
    }
}
