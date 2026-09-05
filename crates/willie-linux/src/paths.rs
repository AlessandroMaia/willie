//! Well-known locations inside the distribution (docs/ARCHITECTURE.md §1.2).

use std::path::{Path, PathBuf};

pub const STATE_DIR: &str = "/var/lib/willie";
pub const RUN_DIR: &str = "/run/willie";
pub const IMAGE_VERSION_FILE: &str = "/etc/willie/image-version";

/// The session supervisor binary the daemon spawns.
pub const SUPERVISOR_BIN: &str = "/opt/willie/bin/willie-sess";

/// Version stamped into the image by `xtask distro build`, if present.
#[must_use]
pub fn image_version() -> Option<String> {
    std::fs::read_to_string(Path::new(IMAGE_VERSION_FILE))
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// Where session sockets live: `<run_dir>/sessions`.
#[must_use]
pub fn sessions_run_dir(run_dir: &Path) -> PathBuf {
    run_dir.join("sessions")
}

/// `<run_dir>/sessions/<id>.sock`.
#[must_use]
pub fn session_socket(run_dir: &Path, id: &str) -> PathBuf {
    sessions_run_dir(run_dir).join(format!("{id}.sock"))
}

/// Where a workspace binary should be run from inside the distribution.
///
/// Cargo bakes in the Windows path of what it built, and running a file
/// straight from the Windows mount is not reliable: measured on
/// 2026-09-05, the identical bytes of one debug binary fault before
/// `main` when executed from the mount and run correctly when copied
/// into the distribution's own filesystem first. So `cargo xtask
/// test-linux` stages the binaries inside the distribution and names
/// that directory in `WILLIE_TEST_BIN_DIR`. Without a stage, fall back
/// to the mount, which is where a developer running one binary by hand
/// still finds it.
#[must_use]
pub fn staged(compiled: &str, stage: Option<&str>) -> String {
    let Some(stage) = stage else {
        return from_windows(compiled);
    };
    let name = compiled
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(compiled)
        .to_owned();
    format!("{}/{name}", stage.trim_end_matches('/'))
}

/// `staged` with the stage the Linux test runner sets, if it did.
#[must_use]
pub fn test_binary(compiled: &str) -> String {
    staged(
        compiled,
        std::env::var("WILLIE_TEST_BIN_DIR").ok().as_deref(),
    )
}

/// A Windows path as the distribution sees it, or the path unchanged
/// when it is not a Windows one.
#[must_use]
pub fn from_windows(path: &str) -> String {
    willie_core::paths::windows_to_drvfs(path)
        .unwrap_or_else(|| path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The staged copy is named by its file name alone, whichever
    /// separator the compiled path came with.
    #[test]
    fn a_stage_replaces_the_directory_and_keeps_the_name() {
        assert_eq!(
            staged(
                r"C:\github\willie\target\debug\willie-sess",
                Some("/tmp/s")
            ),
            "/tmp/s/willie-sess"
        );
        assert_eq!(
            staged("/mnt/c/x/deps/supervisor-abc", Some("/tmp/s/")),
            "/tmp/s/supervisor-abc"
        );
    }

    /// Without a stage the binary is still reachable through the mount,
    /// which is what a developer running one by hand relies on.
    #[test]
    fn without_a_stage_the_windows_path_becomes_a_mount_path() {
        assert_eq!(
            staged(r"C:\github\willie\target\debug\willie", None),
            "/mnt/c/github/willie/target/debug/willie"
        );
        assert_eq!(staged("/usr/bin/already", None), "/usr/bin/already");
    }

    #[test]
    fn a_session_socket_sits_under_the_run_dir() {
        assert_eq!(
            session_socket(Path::new(RUN_DIR), "sess_01J"),
            PathBuf::from("/run/willie/sessions/sess_01J.sock")
        );
    }
}
