//! `check-refs`: versioned content must not mention entries of a denylist.
//!
//! The denylist itself is never versioned. It is read from the file named
//! by `WILLIE_REFS_DENYLIST` or `--list`, defaulting to
//! `docs/blueprint/refs-denylist.txt` (an ignored path). A missing list is
//! an error, not a silent pass: the whole point is to fail closed.
//!
//! Scanned files are exactly those git would commit: tracked files plus
//! untracked files that are not ignored.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::TaskResult;

const DEFAULT_LIST: &str = "docs/blueprint/refs-denylist.txt";

pub fn run(root: &Path, args: &[String]) -> TaskResult {
    let list_path = list_path(root, args)?;
    let terms = load_terms(&list_path)?;
    if terms.is_empty() {
        return Err(format!("{} contains no terms", list_path.display()));
    }
    let files = versioned_files(root)?;
    let mut hits = Vec::new();
    for rel in &files {
        let path = root.join(rel);
        let Ok(bytes) = fs::read(&path) else { continue };
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        hits.extend(scan(rel, &text, &terms));
    }
    if hits.is_empty() {
        println!(
            "check-refs: {} files clean ({} terms)",
            files.len(),
            terms.len()
        );
        Ok(())
    } else {
        for hit in &hits {
            eprintln!("{hit}");
        }
        Err(format!(
            "{} reference(s) to denylisted terms in versioned content",
            hits.len()
        ))
    }
}

fn list_path(root: &Path, args: &[String]) -> Result<PathBuf, String> {
    if let Some(i) = args.iter().position(|a| a == "--list") {
        return args
            .get(i + 1)
            .map(PathBuf::from)
            .ok_or_else(|| "--list needs a path".to_owned());
    }
    if let Ok(env) = std::env::var("WILLIE_REFS_DENYLIST") {
        return Ok(PathBuf::from(env));
    }
    Ok(root.join(DEFAULT_LIST))
}

fn load_terms(path: &Path) -> Result<Vec<String>, String> {
    let text = fs::read_to_string(path).map_err(|e| {
        format!(
            "cannot read denylist {}: {e}\n\
             create it (one term per line, `#` comments) or point \
             WILLIE_REFS_DENYLIST / --list at it",
            path.display()
        )
    })?;
    Ok(parse_terms(&text))
}

fn parse_terms(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_lowercase)
        .collect()
}

fn versioned_files(root: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(root)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Case-insensitive substring search, one report per matching line.
fn scan(rel: &str, text: &str, terms: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let lower = line.to_lowercase();
        for term in terms {
            if lower.contains(term.as_str()) {
                out.push(format!("{rel}:{}: matches `{term}`", index + 1));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_blank_lines_are_not_terms() {
        let terms = parse_terms("# header\n\n  Foo \nbar\n");
        assert_eq!(terms, ["foo", "bar"]);
    }

    #[test]
    fn scan_is_case_insensitive_and_reports_line_numbers() {
        let hits = scan("a.md", "clean\nSee FOO here\n", &["foo".into()]);
        assert_eq!(hits, ["a.md:2: matches `foo`"]);
    }

    #[test]
    fn explicit_list_argument_wins_over_the_default() {
        let root = Path::new("/repo");
        let args = ["--list".to_owned(), "/tmp/list.txt".to_owned()];
        assert_eq!(list_path(root, &args).unwrap(), Path::new("/tmp/list.txt"));
    }
}
