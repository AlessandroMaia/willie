//! The launch spec and the files that sit beside it. The daemon writes
//! `spec.json` once; everything else in the directory is the supervisor's.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use willie_core::session::SessionSpec;

/// Where one session's files live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub dir: PathBuf,
    pub events: PathBuf,
    pub log: PathBuf,
    pub socket: PathBuf,
}

/// Read and validate `spec.json`; derive the sibling paths.
pub fn load(spec_path: &Path) -> io::Result<(SessionSpec, Paths)> {
    let text = fs::read_to_string(spec_path)?;
    let spec: SessionSpec = serde_json::from_str(&text)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if spec.argv.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "spec.argv is empty",
        ));
    }
    let dir = spec_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| io::Error::other("spec path has no parent"))?;
    let paths = Paths {
        events: dir.join("events.jsonl"),
        log: dir.join("supervisor.log"),
        socket: PathBuf::from(&spec.socket),
        dir,
    };
    Ok((spec, paths))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spec_yields_its_sibling_paths_and_the_socket_it_names() {
        let dir = std::env::temp_dir()
            .join(format!("willie-sess-spec-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let text = serde_json::json!({
            "id": "sess_01J00000000000000000000000",
            "project_id": "proj_01J00000000000000000000000",
            "harness": "claude-code",
            "workspace": "/home/willie/projects/x",
            "socket": "/run/willie/sessions/s.sock",
            "argv": ["/bin/true"],
            "env": { "HOME": "/home/willie" },
            "created_at": "1",
            "willie_version": "0.1.0"
        });
        fs::write(dir.join("spec.json"), text.to_string()).unwrap();
        let (spec, paths) = load(&dir.join("spec.json")).unwrap();
        assert_eq!(spec.argv, vec!["/bin/true".to_owned()]);
        assert_eq!(paths.dir, dir);
        assert_eq!(paths.events, dir.join("events.jsonl"));
        assert_eq!(paths.log, dir.join("supervisor.log"));
        assert_eq!(paths.socket, PathBuf::from("/run/willie/sessions/s.sock"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_or_invalid_spec_is_an_error_not_a_panic() {
        assert!(load(Path::new("/nonexistent/spec.json")).is_err());
        let dir = std::env::temp_dir()
            .join(format!("willie-sess-badspec-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("spec.json"), "{ not json").unwrap();
        assert!(load(&dir.join("spec.json")).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
