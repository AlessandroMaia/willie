//! The managed-tools manifest: one TOML file recording what the daemon
//! installed, for display and reinstall after an image migration.
//! Detection, not this file, is the truth the screen shows; the manifest
//! is what Willie knows it installed.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

// Read and written by `tools::list`/`update` (Task 3); allow until then
// so the plain (non-test) binary still builds clean.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRecord {
    pub version: String,
    pub installed_at: String,
    pub installer: String,
}

#[allow(dead_code)]
fn manifest_path(state_dir: &Path) -> PathBuf {
    state_dir.join("tools.toml")
}

/// Every installed tool the daemon has recorded, keyed by tool id. Empty
/// when the file is missing or unreadable — the screen falls back to live
/// detection, so a lost manifest degrades to a re-detect, never an error.
// Called from `tools::list` (Task 3); allow until then.
#[allow(dead_code)]
pub fn load(state_dir: &Path) -> BTreeMap<String, ToolRecord> {
    std::fs::read_to_string(manifest_path(state_dir))
        .ok()
        .and_then(|t| toml::from_str::<BTreeMap<String, ToolRecord>>(&t).ok())
        .unwrap_or_default()
}

/// Merge one tool's record, keeping the rest. A write failure is logged,
/// not surfaced: the install it records already succeeded, and the next
/// install rewrites the file anyway.
// Called from `tools::update` and install (Task 3); allow until then.
#[allow(dead_code)]
pub fn record(state_dir: &Path, id: &str, rec: &ToolRecord) {
    let mut all = load(state_dir);
    all.insert(id.to_owned(), rec.clone());
    let write = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(state_dir)?;
        let text = toml::to_string(&all)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let path = manifest_path(state_dir);
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)
    })();
    if let Err(e) = write {
        eprintln!("willied: could not write the tools manifest: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("willie-manifest-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn record_then_load_round_trips_an_entry() {
        let dir = scratch("rt");
        record(
            &dir,
            "claude-code",
            &ToolRecord {
                version: "2.1.246".into(),
                installed_at: "1757000000".into(),
                installer: "curl … | bash".into(),
            },
        );
        let loaded = load(&dir);
        assert_eq!(
            loaded.get("claude-code").map(|r| r.version.as_str()),
            Some("2.1.246")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_loads_empty() {
        assert!(load(&scratch("missing")).is_empty());
    }

    #[test]
    fn an_unreadable_file_loads_empty() {
        let dir = scratch("bad");
        std::fs::write(dir.join("tools.toml"), "not = valid = toml").unwrap();
        assert!(load(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recording_a_second_tool_keeps_the_first() {
        let dir = scratch("two");
        record(
            &dir,
            "a",
            &ToolRecord {
                version: "1".into(),
                installed_at: "1".into(),
                installer: "x".into(),
            },
        );
        record(
            &dir,
            "b",
            &ToolRecord {
                version: "2".into(),
                installed_at: "2".into(),
                installer: "y".into(),
            },
        );
        let loaded = load(&dir);
        assert_eq!(loaded.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
