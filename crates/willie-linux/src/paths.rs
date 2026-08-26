//! Well-known locations inside the distribution (see docs/ARCHITECTURE.md §1.2).

use std::path::Path;

pub const STATE_DIR: &str = "/var/lib/willie";
pub const RUN_DIR: &str = "/run/willie";
pub const IMAGE_VERSION_FILE: &str = "/etc/willie/image-version";

/// Version stamped into the image by `xtask distro build`, if present.
#[must_use]
pub fn image_version() -> Option<String> {
    std::fs::read_to_string(Path::new(IMAGE_VERSION_FILE))
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}
