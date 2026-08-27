//! `engine.toml` — the only Windows-side state besides the image. Holds
//! the machine profile and UI preferences; this slice adds project roots.

use std::{fs, io, path::Path};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineConfig {
    #[serde(default)]
    pub projects: Projects,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Projects {
    #[serde(default)]
    pub roots: Vec<String>,
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
}
