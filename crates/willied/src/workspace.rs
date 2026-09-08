//! Workspace-relative path resolution and read-only tree/file helpers.
//! `resolve_within` is the *only* place a workspace-relative path becomes
//! a filesystem path; both `project.tree` and `project.read_file` go
//! through it, so a containment bug here is a containment bug everywhere.

use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

use willie_proto::project::{EntryKind, TreeEntry};

/// Reads at most this many bytes of a file; one more byte than this
/// signals `truncated: true` without reading the whole file first.
const READ_CAP: usize = 512 * 1024;

/// How much of a file's head is sniffed for a NUL byte before it is
/// treated as binary.
const SNIFF_LEN: usize = 8 * 1024;

/// A workspace containment or read failure. Never carries a raw path in
/// its `Debug` form the caller did not already know, since the wire
/// message is built from it separately in `handlers.rs`.
#[derive(Debug)]
pub enum WsError {
    /// `rel` was absolute, contained a `..` component, or resolved
    /// (after canonicalisation, so a symlink cannot hide it) outside the
    /// workspace.
    Outside,
    /// The file's first [`SNIFF_LEN`] bytes contain a NUL.
    NotText,
    /// Reading the filesystem failed for some other reason.
    Io(std::io::Error),
}

impl WsError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Outside => "path_outside_workspace",
            Self::NotText => "file_not_text",
            Self::Io(_) => "workspace_read_failed",
        }
    }

    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Outside => {
                "the path is outside the project's workspace".to_owned()
            }
            Self::NotText => "the file is not text".to_owned(),
            Self::Io(e) => e.to_string(),
        }
    }

    #[must_use]
    pub fn remediation(&self) -> &'static str {
        match self {
            Self::Outside => "name a path inside the project's workspace",
            Self::NotText => "this file is binary; open it in VS Code instead",
            Self::Io(_) => "check the workspace and try again",
        }
    }
}

/// Resolves a caller-supplied, workspace-relative path to a filesystem
/// path, refusing anything that could name a file outside `workspace`:
/// an absolute `rel`, any `..` component, or — since a symlink can make
/// an innocent-looking relative path escape anyway — a canonicalised
/// result that does not start with the canonicalised workspace. On
/// Windows hosts `Path::is_absolute()` does not recognise a rooted path
/// like `/etc/passwd` and `join` replaces the root component wholesale,
/// so the canonicalised `starts_with` check below is the one guard that
/// must never be removed — the other two are defence in depth, not this
/// function's real gate.
pub fn resolve_within(workspace: &Path, rel: &str) -> Result<PathBuf, WsError> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return Err(WsError::Outside);
    }
    if rel_path
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(WsError::Outside);
    }

    let joined = workspace.join(rel_path);
    let canonical_workspace =
        fs::canonicalize(workspace).map_err(WsError::Io)?;
    let canonical_target = fs::canonicalize(&joined).map_err(WsError::Io)?;
    if !canonical_target.starts_with(&canonical_workspace) {
        return Err(WsError::Outside);
    }
    Ok(canonical_target)
}

/// Parses `git status --porcelain=v1 --untracked-files=all` into a map
/// of workspace-relative path to a one-letter flag: `?` for an untracked
/// path, `R` for a rename (keyed by the *new* path), else the first
/// non-space of the two-character `XY` status (`M`/`A`/`D`/…).
#[must_use]
pub fn parse_porcelain(v1_output: &str) -> BTreeMap<String, String> {
    let mut flags = BTreeMap::new();
    for line in v1_output.lines() {
        // `get` rather than byte-slicing: a line whose first bytes are
        // not ASCII (an unexpected, malformed `git status` line) would
        // otherwise panic slicing mid-character. Such a line is simply
        // skipped, not a crash.
        let (Some(xy), Some(rest)) = (line.get(..2), line.get(3..)) else {
            continue;
        };
        if xy == "??" {
            flags.insert(rest.to_owned(), "?".to_owned());
            continue;
        }
        if xy.starts_with('R') || xy.ends_with('R') {
            let new_path = rest.split_once(" -> ").map_or(rest, |(_, new)| new);
            flags.insert(new_path.to_owned(), "R".to_owned());
            continue;
        }
        let flag = xy.chars().find(|c| *c != ' ').unwrap_or('M');
        flags.insert(rest.to_owned(), flag.to_string());
    }
    flags
}

/// Lists one directory level: directories first, then files, each group
/// sorted case-insensitively by name; `.git` is skipped. A directory's
/// `git` flag is the flag of any changed path starting with
/// `<rel_prefix>/<name>/`, aggregated to a single `"M"` when more than
/// one kind of change is beneath it; a file's flag is its own entry in
/// `flags`, keyed by `<rel_prefix>/<name>` (or `<name>` at the root).
#[must_use]
pub fn list_dir(
    dir: &Path,
    flags: &BTreeMap<String, String>,
    rel_prefix: &str,
) -> Vec<TreeEntry> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let rel = if rel_prefix.is_empty() {
            name.clone()
        } else {
            format!("{rel_prefix}/{name}")
        };
        if file_type.is_dir() {
            let prefix = format!("{rel}/");
            let git = if flags.keys().any(|p| p.starts_with(&prefix)) {
                Some("M".to_owned())
            } else {
                None
            };
            dirs.push(TreeEntry {
                name,
                kind: EntryKind::Dir,
                git,
            });
        } else {
            let git = flags.get(&rel).cloned();
            files.push(TreeEntry {
                name,
                kind: EntryKind::File,
                git,
            });
        }
    }

    dirs.sort_by_key(|e| e.name.to_lowercase());
    files.sort_by_key(|e| e.name.to_lowercase());
    dirs.extend(files);
    dirs
}

/// Reads a file as text, capped at [`READ_CAP`]: a NUL byte in the first
/// [`SNIFF_LEN`] bytes refuses it as binary before the cap is even
/// applied; past the cap the content is cut and `truncated` is `true`.
/// Invalid UTF-8 sequences are replaced, never refused. Refuses anything
/// that is not a regular file — a directory, or a FIFO a build script or
/// the agent left in the workspace — as `NotText` *before* ever opening
/// it: `fs::metadata` is a `stat`, which never blocks, while `File::open`
/// on a FIFO with no writer blocks forever and would wedge the daemon's
/// one-request-at-a-time dispatch loop for good.
pub fn read_text(path: &Path) -> Result<(String, bool), WsError> {
    let meta = fs::metadata(path).map_err(WsError::Io)?;
    if !meta.is_file() {
        return Err(WsError::NotText);
    }
    let mut file = fs::File::open(path).map_err(WsError::Io)?;
    let mut buf = Vec::with_capacity(SNIFF_LEN.min(READ_CAP));
    (&mut file)
        .take(SNIFF_LEN as u64)
        .read_to_end(&mut buf)
        .map_err(WsError::Io)?;
    if buf.contains(&0) {
        return Err(WsError::NotText);
    }
    let remaining = (READ_CAP - buf.len()) as u64 + 1;
    (&mut file)
        .take(remaining)
        .read_to_end(&mut buf)
        .map_err(WsError::Io)?;

    let truncated = buf.len() > READ_CAP;
    buf.truncate(READ_CAP);
    let content = String::from_utf8_lossy(&buf).into_owned();
    Ok((content, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "willie-workspace-test-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_within_refuses_dot_dot_absolute_and_a_symlink_escape() {
        let root = scratch("escape");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();

        assert!(matches!(
            resolve_within(&workspace, "../outside"),
            Err(WsError::Outside)
        ));
        assert!(matches!(
            resolve_within(&workspace, "sub/../../escape"),
            Err(WsError::Outside)
        ));
        // An absolute `rel` is refused on every host; `workspace` itself
        // is already absolute (built from `std::env::temp_dir()`), so
        // its own string is an absolute path on Windows and Unix alike
        // without hardcoding either platform's root syntax.
        assert!(matches!(
            resolve_within(&workspace, &workspace.to_string_lossy()),
            Err(WsError::Outside)
        ));

        #[cfg(unix)]
        {
            let sibling = root.join("sibling");
            fs::create_dir_all(&sibling).unwrap();
            fs::write(sibling.join("secret.txt"), "s").unwrap();
            std::os::unix::fs::symlink(&sibling, workspace.join("link"))
                .unwrap();
            assert!(matches!(
                resolve_within(&workspace, "link/secret.txt"),
                Err(WsError::Outside)
            ));
        }

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_within_accepts_a_nested_relative_path() {
        let root = scratch("nested");
        let workspace = root.join("workspace");
        fs::create_dir_all(workspace.join("a/b")).unwrap();
        fs::write(workspace.join("a/b/c.txt"), "hi").unwrap();

        let resolved = resolve_within(&workspace, "a/b/c.txt").unwrap();
        assert_eq!(
            resolved,
            fs::canonicalize(workspace.join("a/b/c.txt")).unwrap()
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn parse_porcelain_maps_modified_added_deleted_renamed_and_untracked() {
        let output = " M src/modified.rs\n\
                       A  src/added.rs\n\
                       D  src/deleted.rs\n\
                       R  src/old.rs -> src/new.rs\n\
                       ?? src/new_file.rs\n\
                       \u{1F600}?? garbage.rs\n";
        let flags = parse_porcelain(output);

        assert_eq!(flags.get("src/modified.rs"), Some(&"M".to_owned()));
        assert_eq!(flags.get("src/added.rs"), Some(&"A".to_owned()));
        assert_eq!(flags.get("src/deleted.rs"), Some(&"D".to_owned()));
        assert_eq!(flags.get("src/new.rs"), Some(&"R".to_owned()));
        assert!(!flags.contains_key("src/old.rs"));
        assert_eq!(flags.get("src/new_file.rs"), Some(&"?".to_owned()));
        // A line whose first character is multibyte and does not land on
        // a 2-byte boundary is skipped, not a byte-slicing panic.
        assert_eq!(flags.len(), 5);
    }

    #[test]
    fn list_dir_puts_directories_first_skips_git_and_flags_a_dir_with_a_changed_child()
     {
        let root = scratch("list");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("zdir")).unwrap();
        fs::create_dir_all(root.join("adir")).unwrap();
        fs::write(root.join("zdir/changed.rs"), "x").unwrap();
        fs::write(root.join("readme.md"), "x").unwrap();
        fs::write(root.join("apple.txt"), "x").unwrap();

        let mut flags = BTreeMap::new();
        flags.insert("zdir/changed.rs".to_owned(), "M".to_owned());

        let entries = list_dir(&root, &flags, "");

        assert!(!entries.iter().any(|e| e.name == ".git"));
        // Directories first, both sorted case-insensitively.
        assert_eq!(entries[0].name, "adir");
        assert_eq!(entries[0].kind, EntryKind::Dir);
        assert_eq!(entries[0].git, None);
        assert_eq!(entries[1].name, "zdir");
        assert_eq!(entries[1].kind, EntryKind::Dir);
        assert_eq!(entries[1].git, Some("M".to_owned()));
        // Files after directories, sorted case-insensitively.
        let file_names: Vec<&str> =
            entries[2..].iter().map(|e| e.name.as_str()).collect();
        assert_eq!(file_names, vec!["apple.txt", "readme.md"]);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_text_refuses_a_binary_and_truncates_past_the_cap() {
        let root = scratch("read");
        let binary = root.join("bin.dat");
        fs::write(&binary, [b'a', b'b', 0, b'c']).unwrap();
        let err = read_text(&binary).unwrap_err();
        assert_eq!(err.code(), "file_not_text");

        let big = root.join("big.txt");
        let content = "a".repeat(READ_CAP + 1024);
        fs::write(&big, &content).unwrap();
        let (read, truncated) = read_text(&big).unwrap();
        assert!(truncated);
        assert_eq!(read.len(), READ_CAP);

        let small = root.join("small.txt");
        fs::write(&small, "hello").unwrap();
        let (read, truncated) = read_text(&small).unwrap();
        assert_eq!(read, "hello");
        assert!(!truncated);

        let _ = fs::remove_dir_all(&root);
    }

    /// A directory (or any non-regular-file path) is refused as
    /// `NotText` from `fs::metadata` alone, without ever calling
    /// `File::open` on it — the same stat-first guard that keeps a FIFO
    /// from blocking the read forever.
    #[test]
    fn read_text_refuses_a_directory_as_not_text() {
        let root = scratch("read-dir");
        let err = read_text(&root).unwrap_err();
        assert_eq!(err.code(), "file_not_text");

        let _ = fs::remove_dir_all(&root);
    }
}
