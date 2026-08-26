//! Windows-side locations and Windows → WSL path mapping.

use std::path::{Component, Path, PathBuf, Prefix};

/// `%LOCALAPPDATA%\Willie` — the only Windows-side state directory.
#[must_use]
pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|base| PathBuf::from(base).join("Willie"))
}

/// Maps an absolute Windows path with a drive letter to the DrvFs mount
/// WSL exposes it at (`C:\x\y` → `/mnt/c/x/y`). Returns `None` for
/// relative paths, UNC paths and `\\wsl.localhost` paths.
#[must_use]
pub fn to_wsl_path(windows: &Path) -> Option<String> {
    let mut components = windows.components();
    let drive = match components.next()? {
        Component::Prefix(prefix) => match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                letter.to_ascii_lowercase() as char
            }
            _ => return None,
        },
        _ => return None,
    };
    let mut out = format!("/mnt/{drive}");
    for component in components {
        match component {
            Component::RootDir => {}
            Component::Normal(part) => {
                out.push('/');
                out.push_str(&part.to_string_lossy());
            }
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_letter_path_maps_to_mnt() {
        assert_eq!(
            to_wsl_path(Path::new(r"C:\github\willie")).as_deref(),
            Some("/mnt/c/github/willie")
        );
    }

    #[test]
    fn drive_letter_is_lowercased_and_root_is_handled() {
        assert_eq!(to_wsl_path(Path::new(r"D:\")).as_deref(), Some("/mnt/d"));
    }

    #[test]
    fn relative_and_unc_paths_are_rejected() {
        assert_eq!(to_wsl_path(Path::new(r"docs\x")), None);
        assert_eq!(to_wsl_path(Path::new(r"\\server\share\x")), None);
    }

    #[cfg(windows)]
    #[test]
    fn data_dir_is_willie_under_localappdata() {
        let dir = data_dir().expect("LOCALAPPDATA is always set on Windows");
        assert!(dir.ends_with("Willie"), "unexpected data dir {dir:?}");
    }
}
