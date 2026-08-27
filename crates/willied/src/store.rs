//! Projects persist as one TOML file per project under the state dir.
//! These files are the truth; the in-memory index is rebuilt from them.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use willie_core::{id::ProjectId, project::Project};

#[must_use]
pub fn projects_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("projects")
}

pub fn load_all(state_dir: &Path) -> Vec<Project> {
    let dir = projects_dir(state_dir);
    let mut out = Vec::new();
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        match fs::read_to_string(&path)
            .ok()
            .and_then(|t| toml::from_str::<Project>(&t).ok())
        {
            Some(p) => out.push(p),
            None => eprintln!(
                "willied: skipping unreadable project {}",
                path.display()
            ),
        }
    }
    out
}

pub fn save(state_dir: &Path, p: &Project) -> io::Result<()> {
    let dir = projects_dir(state_dir);
    fs::create_dir_all(&dir)?;
    let text =
        toml::to_string(p).map_err(|e| io::Error::other(e.to_string()))?;
    let final_path = dir.join(format!("{}.toml", p.id));
    let tmp = dir.join(format!("{}.toml.tmp", p.id));
    fs::write(&tmp, text)?;
    fs::rename(&tmp, &final_path)
}

/// Persists `p`, logging a write failure to stderr instead of
/// discarding it. In-memory state stays the truth for as long as the
/// daemon runs; a failed write here only leaves a stale copy on disk
/// for the next restart to re-read.
pub(crate) fn save_or_log(state_dir: &Path, p: &Project) {
    if let Err(e) = save(state_dir, p) {
        eprintln!(
            "willied: could not persist project {} ({}): {}",
            p.id, p.slug, e
        );
    }
}

pub fn delete(state_dir: &Path, id: &ProjectId) -> io::Result<()> {
    let path = projects_dir(state_dir).join(format!("{id}.toml"));
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use willie_core::project::ProjectState;

    fn sample(slug: &str) -> Project {
        Project {
            id: ProjectId::new(),
            name: slug.to_owned(),
            slug: slug.to_owned(),
            source: format!(r"C:\x\{slug}"),
            workspace: format!("/home/willie/projects/{slug}"),
            branch: "main".into(),
            state: ProjectState::Ready,
            source_present: true,
            created_at: "t".into(),
        }
    }

    #[test]
    fn save_then_load_round_trips_and_delete_removes() {
        let dir = std::env::temp_dir()
            .join(format!("willie-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let a = sample("alpha");
        let b = sample("beta");
        save(&dir, &a).unwrap();
        save(&dir, &b).unwrap();
        let mut loaded = load_all(&dir);
        loaded.sort_by(|x, y| x.slug.cmp(&y.slug));
        assert_eq!(loaded, vec![a.clone(), b.clone()]);
        delete(&dir, &a.id).unwrap();
        let after = load_all(&dir);
        assert_eq!(after, vec![b]);
        // deleting a missing id is not an error
        delete(&dir, &a.id).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_file_is_skipped_not_fatal() {
        let dir = std::env::temp_dir()
            .join(format!("willie-store-bad-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(projects_dir(&dir)).unwrap();
        fs::write(projects_dir(&dir).join("junk.toml"), "not = valid = toml")
            .unwrap();
        save(&dir, &sample("ok")).unwrap();
        assert_eq!(load_all(&dir).len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }
}
