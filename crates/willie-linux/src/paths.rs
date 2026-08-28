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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_socket_sits_under_the_run_dir() {
        assert_eq!(
            session_socket(Path::new(RUN_DIR), "sess_01J"),
            PathBuf::from("/run/willie/sessions/sess_01J.sock")
        );
    }
}
