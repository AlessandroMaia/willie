//! Find git repositories under user-configured root folders.

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Candidate {
    pub path: String,
    pub name: String,
}

#[must_use]
pub fn discover(roots: &[String], max_depth: usize) -> Vec<Candidate> {
    let mut out = Vec::new();
    for root in roots {
        walk(Path::new(root), max_depth, &mut out);
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out.dedup();
    out
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<Candidate>) {
    if dir.join(".git").exists() {
        if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
            out.push(Candidate {
                path: dir.to_string_lossy().into_owned(),
                name: name.to_owned(),
            });
        }
        return; // do not descend into a repository
    }
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            walk(&entry.path(), depth - 1, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_repositories_within_the_depth_limit() {
        let root = std::env::temp_dir()
            .join(format!("willie-disc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("group/repo/.git")).unwrap();
        fs::create_dir_all(root.join("group/repo/src")).unwrap();
        fs::create_dir_all(root.join("plain")).unwrap();
        fs::create_dir_all(root.join("a/b/c/deep/.git")).unwrap();
        let found = discover(&[root.to_string_lossy().into_owned()], 2);
        let names: Vec<&str> = found.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"repo"));
        assert!(!names.contains(&"deep"), "beyond depth 2");
        let _ = fs::remove_dir_all(&root);
    }
}
