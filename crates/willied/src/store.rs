//! Projects persist as one TOML file per project under the state dir.
//! These files are the truth; the in-memory index is rebuilt from them.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use willie_core::{
    id::ProjectId,
    project::{Project, SandboxProblem},
    sandbox::SandboxProfile,
};

#[must_use]
pub fn projects_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("projects")
}

/// Parses each project file in two stages. The record's `[sandbox]`
/// table is a `deny_unknown_fields` `SandboxProfile`, hand-edited by
/// people, so one mistyped key there must not fail the whole record: a
/// record that parses as a bare TOML table but whose `[sandbox]`
/// sub-table does not become a `SandboxProfile` loads with the default
/// profile and a `sandbox_problem` recording why. A record that fails
/// to parse at all, or whose *other* fields do not match `Project`, is
/// still skipped as before.
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
        let Ok(text) = fs::read_to_string(&path) else {
            eprintln!(
                "willied: skipping unreadable project {}",
                path.display()
            );
            continue;
        };
        let path_display = path.display().to_string();
        let Ok(mut table) = toml::from_str::<toml::Table>(&text) else {
            eprintln!("willied: skipping unreadable project {path_display}");
            continue;
        };
        let raw_sandbox = table.remove("sandbox");
        let sandbox_problem = match &raw_sandbox {
            Some(v) => match v.clone().try_into::<SandboxProfile>() {
                Ok(_) => None,
                Err(e) => Some(SandboxProblem {
                    code: "sandbox_profile_invalid".to_owned(),
                    message: format!(
                        "the [sandbox] table could not be read: {e}"
                    ),
                    remediation: "open Sandbox… for this project and save \
                                  it to replace the table, or fix the file"
                        .to_owned(),
                }),
            },
            None => None,
        };
        if sandbox_problem.is_none()
            && let Some(v) = raw_sandbox
        {
            table.insert("sandbox".to_owned(), v);
        }
        let Ok(mut project) = table.try_into::<Project>() else {
            eprintln!("willied: skipping unreadable project {path_display}");
            continue;
        };
        project.sandbox_problem = sandbox_problem;
        out.push(project);
    }
    out
}

/// Writes `p`'s record. A project loaded with an unreadable `[sandbox]`
/// table carries the default profile in memory, not the person's own
/// table, so an unrelated save (a rename, a job's completion, …) must
/// not overwrite that table with the default until its owner replaces
/// it via `set_sandbox`.
pub fn save(state_dir: &Path, p: &Project) -> io::Result<()> {
    let dir = projects_dir(state_dir);
    fs::create_dir_all(&dir)?;
    let final_path = dir.join(format!("{}.toml", p.id));
    let mut doc = toml::Table::try_from(p)
        .map_err(|e| io::Error::other(e.to_string()))?;
    if p.sandbox_problem.is_some()
        && let Ok(text) = fs::read_to_string(&final_path)
        && let Ok(mut old) = toml::from_str::<toml::Table>(&text)
        && let Some(sandbox) = old.remove("sandbox")
    {
        doc.insert("sandbox".to_owned(), sandbox);
    }
    doc.remove("sandbox_problem"); // recomputed on load, not persisted as truth
    let text =
        toml::to_string(&doc).map_err(|e| io::Error::other(e.to_string()))?;
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

/// Forgets `p`'s file, logging an unlink failure to stderr instead of
/// discarding it. The removal stands in memory either way, but a file
/// left behind resurrects the project on the next daemon start, so the
/// failure has to be visible somewhere.
pub(crate) fn delete_or_log(state_dir: &Path, p: &Project) {
    if let Err(e) = delete(state_dir, &p.id) {
        eprintln!(
            "willied: could not forget project {} ({}): {}",
            p.id, p.slug, e
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use willie_core::{project::ProjectState, sandbox::SandboxProfile};

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
            sandbox: SandboxProfile::default(),
            sandbox_problem: None,
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

    #[test]
    fn an_unknown_sandbox_key_loads_the_project_with_a_problem() {
        let dir = std::env::temp_dir()
            .join(format!("willie-store-bad-sandbox-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(projects_dir(&dir)).unwrap();
        let id = ProjectId::new();
        let toml = format!(
            "id = \"{id}\"\nname = \"x\"\nslug = \"x\"\n\
             source = \"C:\\\\x\"\nworkspace = \"/home/willie/projects/x\"\n\
             branch = \"main\"\ncreated_at = \"t\"\n\
             [state]\nstate = \"ready\"\n\
             [sandbox]\nnonsense = true\n"
        );
        fs::write(projects_dir(&dir).join(format!("{id}.toml")), toml).unwrap();

        let loaded = load_all(&dir);

        assert_eq!(loaded.len(), 1);
        let p = &loaded[0];
        assert_eq!(p.sandbox, SandboxProfile::default());
        let problem = p.sandbox_problem.as_ref().expect("a problem");
        assert_eq!(problem.code, "sandbox_profile_invalid");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_valid_sandbox_table_loads_without_a_problem() {
        let dir = std::env::temp_dir()
            .join(format!("willie-store-good-sandbox-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(projects_dir(&dir)).unwrap();
        let id = ProjectId::new();
        let toml = format!(
            "id = \"{id}\"\nname = \"x\"\nslug = \"x\"\n\
             source = \"C:\\\\x\"\nworkspace = \"/home/willie/projects/x\"\n\
             branch = \"main\"\ncreated_at = \"t\"\n\
             [state]\nstate = \"ready\"\n\
             [sandbox]\nagent_state = false\n"
        );
        fs::write(projects_dir(&dir).join(format!("{id}.toml")), toml).unwrap();

        let loaded = load_all(&dir);

        assert_eq!(loaded.len(), 1);
        let p = &loaded[0];
        assert_eq!(p.sandbox_problem, None);
        assert_eq!(
            p.sandbox,
            SandboxProfile {
                agent_state: Some(false),
                ..SandboxProfile::default()
            }
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_save_preserves_an_unreadable_sandbox_table() {
        let dir = std::env::temp_dir().join(format!(
            "willie-store-preserve-sandbox-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(projects_dir(&dir)).unwrap();
        let id = ProjectId::new();
        let path = projects_dir(&dir).join(format!("{id}.toml"));
        let toml = format!(
            "id = \"{id}\"\nname = \"x\"\nslug = \"x\"\n\
             source = \"C:\\\\x\"\nworkspace = \"/home/willie/projects/x\"\n\
             branch = \"main\"\ncreated_at = \"t\"\n\
             [state]\nstate = \"ready\"\n\
             [sandbox]\nnonsense = true\n"
        );
        fs::write(&path, toml).unwrap();

        let loaded = load_all(&dir);
        assert_eq!(loaded.len(), 1);
        let p = loaded[0].clone();
        assert!(p.sandbox_problem.is_some());

        // An unrelated save (the in-memory project holds the default
        // profile, not the hand-written table) must not clobber it.
        save(&dir, &p).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("nonsense = true"), "{text}");
        let _ = fs::remove_dir_all(&dir);
    }
}
