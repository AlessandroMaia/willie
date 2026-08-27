//! Pure mapping from a Windows path to the DrvFs mount WSL exposes it at.

/// `C:\a\b` → `/mnt/c/a/b`. Returns `None` for a relative path, a UNC or
/// `\\wsl$` path, or anything without a drive letter.
#[must_use]
pub fn windows_to_drvfs(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    if bytes.len() < 3 || bytes[1] != b':' {
        return None;
    }
    let drive = (bytes[0] as char).to_ascii_lowercase();
    if !drive.is_ascii_alphabetic() {
        return None;
    }
    let sep = bytes[2];
    if sep != b'\\' && sep != b'/' {
        return None;
    }
    let rest = path[2..].replace('\\', "/");
    Some(format!("/mnt/{drive}{rest}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_a_drive_letter_path_to_its_mount() {
        assert_eq!(
            windows_to_drvfs(r"C:\github\x"),
            Some("/mnt/c/github/x".to_owned())
        );
        assert_eq!(
            windows_to_drvfs(r"D:\a b\c"),
            Some("/mnt/d/a b/c".to_owned())
        );
    }

    #[test]
    fn rejects_paths_without_a_drive_letter() {
        assert_eq!(windows_to_drvfs(r"\\server\share"), None);
        assert_eq!(windows_to_drvfs(r"relative\path"), None);
        assert_eq!(windows_to_drvfs(""), None);
    }
}
