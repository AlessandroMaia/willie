//! Windows-side locations and Windows → WSL path mapping.

use std::path::{Component, Path, PathBuf, Prefix};

use willie_core::paths::windows_to_drvfs;

/// `%LOCALAPPDATA%\Willie\data` — the only Windows-side state directory.
/// It sits inside the per-user install root (`%LOCALAPPDATA%\Willie`)
/// in a subdirectory the installer never touches, so uninstalling or
/// reinstalling the app leaves engine state and the distribution disk
/// alone.
#[must_use]
pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|base| PathBuf::from(base).join("Willie").join("data"))
}

/// `engine.toml`, the machine profile and UI preferences file. It sits
/// next to the distro install directory under [`data_dir`], so removing
/// the data directory removes both together.
#[must_use]
pub fn engine_toml_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("engine.toml"))
}

/// Maps an absolute Windows path with a drive letter to the DrvFs mount
/// WSL exposes it at (`C:\x\y` → `/mnt/c/x/y`). Returns `None` for
/// relative paths, UNC paths and `\\wsl.localhost` paths.
///
/// The drive-letter-to-`/mnt` string mapping is
/// [`willie_core::paths::windows_to_drvfs`] — the one pure mapping, also
/// used inside the daemon. This function's own job is to recognize
/// `Path`'s Windows-specific prefix kinds (a plain disk or a `\\?\`
/// verbatim disk) and to normalize components into the canonical
/// `C:\a\b` string `windows_to_drvfs` expects, then hand off to it.
#[must_use]
pub fn to_wsl_path(windows: &Path) -> Option<String> {
    let mut components = windows.components();
    let drive = match components.next()? {
        Component::Prefix(prefix) => match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                letter as char
            }
            _ => return None,
        },
        _ => return None,
    };
    let mut canonical = format!("{drive}:\\");
    for component in components {
        match component {
            Component::RootDir => {}
            Component::Normal(part) => {
                if !canonical.ends_with('\\') {
                    canonical.push('\\');
                }
                canonical.push_str(&part.to_string_lossy());
            }
            _ => return None,
        }
    }
    // `windows_to_drvfs` carries the root separator straight into its
    // output, so a bare drive root ("D:\") maps to "/mnt/d/" rather
    // than "/mnt/d"; trim that back to the canonical mount path.
    windows_to_drvfs(&canonical).map(|mnt| mnt.trim_end_matches('/').to_owned())
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

    #[test]
    fn verbatim_disk_prefix_maps_like_a_plain_disk() {
        assert_eq!(
            to_wsl_path(Path::new(r"\\?\C:\github\willie")).as_deref(),
            Some("/mnt/c/github/willie")
        );
    }

    #[cfg(windows)]
    #[test]
    fn data_dir_is_willie_data_under_localappdata() {
        let dir = data_dir().expect("LOCALAPPDATA is always set on Windows");
        assert!(dir.ends_with(r"Willie\data"), "unexpected data dir {dir:?}");
    }
}
