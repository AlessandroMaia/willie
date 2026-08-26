//! Engine ownership inside the Tauri app: one `Engine` behind a mutex,
//! and the list of places the distribution image may live.

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use willie_engine::Engine;

pub struct EngineState(pub Mutex<Engine>);

pub const IMAGE_FILE: &str = "willie-rootfs.tar.gz";

/// Where to look for the image, most specific first.
pub fn image_candidates(
    env_override: Option<PathBuf>,
    resource_dir: Option<PathBuf>,
    workspace_root: &Path,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(explicit) = env_override {
        out.push(explicit);
    }
    if let Some(dir) = resource_dir {
        out.push(dir.join(IMAGE_FILE));
    }
    out.push(
        workspace_root
            .join("target")
            .join("distro")
            .join(IMAGE_FILE),
    );
    out
}

/// The repository root when running from `cargo tauri dev`
/// (`apps/willie-app/src-tauri` → three levels up); harmless in a bundle.
/// Cargo sets `CARGO_MANIFEST_DIR` at run time under `cargo run`, so a
/// moved checkout is found without a rebuild; the compiled-in value is
/// the fallback for the bundled binary, where the path goes unused.
pub fn workspace_root() -> PathBuf {
    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    manifest_dir
        .ancestors()
        .nth(3)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_root_holds_the_workspace_manifest() {
        let manifest = workspace_root().join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest).unwrap();
        assert!(text.contains("[workspace]"), "{}", manifest.display());
    }

    #[test]
    fn explicit_env_override_comes_first_then_resources_then_dev_target() {
        let list = image_candidates(
            Some(PathBuf::from(r"C:\x\override.tar.gz")),
            Some(PathBuf::from(r"C:\app\resources")),
            Path::new(r"C:\repo"),
        );
        assert_eq!(list[0], PathBuf::from(r"C:\x\override.tar.gz"));
        assert_eq!(
            list[1],
            PathBuf::from(r"C:\app\resources\willie-rootfs.tar.gz")
        );
        assert_eq!(
            list[2],
            PathBuf::from(r"C:\repo\target\distro\willie-rootfs.tar.gz")
        );
    }
}
