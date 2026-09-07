//! On-disk session directories under `<state>/sessions/<id>/`: writing the
//! immutable spec, and reading spec + events back when the daemon scans
//! at start.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use willie_core::session::{SessionEvent, SessionSpec};

/// `<state>/sessions`.
#[must_use]
pub fn sessions_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("sessions")
}

/// `<state>/sessions/<id>`.
#[must_use]
pub fn session_dir(state_dir: &Path, id: &str) -> PathBuf {
    sessions_dir(state_dir).join(id)
}

/// Write `spec.json` into a fresh session directory.
pub fn write_spec(state_dir: &Path, spec: &SessionSpec) -> io::Result<PathBuf> {
    let dir = session_dir(state_dir, &spec.id.to_string());
    fs::create_dir_all(&dir)?;
    let text = serde_json::to_string_pretty(spec)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::write(dir.join("spec.json"), text)?;
    Ok(dir)
}

/// Every session directory's spec and current event log, for the scan.
#[must_use]
pub fn load_all(state_dir: &Path) -> Vec<(SessionSpec, Vec<SessionEvent>)> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(sessions_dir(state_dir)) else {
        return out;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let Ok(text) = fs::read_to_string(dir.join("spec.json")) else {
            continue;
        };
        let Ok(spec) = serde_json::from_str::<SessionSpec>(&text) else {
            continue;
        };
        let events = read_events(&dir.join("events.jsonl"));
        out.push((spec, events));
    }
    out
}

fn read_events(path: &Path) -> Vec<SessionEvent> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests {
    use std::collections::BTreeMap;

    use willie_core::{
        id::{ProjectId, SessionId},
        session::{SessionEvent, SessionEventKind, SessionKind, SessionSpec},
    };

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir()
            .join(format!("willie-sstore-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    fn spec(id: SessionId) -> SessionSpec {
        SessionSpec {
            id,
            project_id: ProjectId::new(),
            harness: "claude-code".into(),
            workspace: "/w".into(),
            socket: "/run/willie/sessions/s.sock".into(),
            argv: vec!["/bin/true".into()],
            env: BTreeMap::new(),
            created_at: "1".into(),
            willie_version: "0.1.0".into(),
            resumed_from: None,
            kind: SessionKind::Agent,
            capabilities: willie_core::sandbox::CapabilitySet::default(),
        }
    }

    #[test]
    fn write_spec_then_load_all_round_trips_the_spec_and_events() {
        let root = scratch("roundtrip");
        let state_dir = root.join("state");
        let id = SessionId::new();
        let dir = write_spec(&state_dir, &spec(id)).unwrap();
        // A well-formed event line and a garbage one: only the first
        // survives the load.
        let good = serde_json::to_string(&SessionEvent {
            at: "2".into(),
            kind: SessionEventKind::Started { pid: 9 },
        })
        .unwrap();
        fs::write(dir.join("events.jsonl"), format!("{good}\nnot json\n"))
            .unwrap();
        let all = load_all(&state_dir);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0.id, id);
        assert_eq!(all[0].1.len(), 1);
        assert!(matches!(
            all[0].1[0].kind,
            SessionEventKind::Started { pid: 9 }
        ));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn load_all_is_empty_when_there_are_no_session_directories() {
        let root = scratch("empty");
        assert!(load_all(&root.join("state")).is_empty());
    }
}
