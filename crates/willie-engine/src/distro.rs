//! The Willie distribution as seen from Windows: registered? running?
//! install or remove it from the bundled image.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{error::WslError, paths::data_dir, wsl::WslCli};

pub const DISTRO_NAME: &str = "willie";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistroStatus {
    pub registered: bool,
    pub running: bool,
    pub install_dir: String,
}

#[must_use]
pub fn install_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("distro"))
}

/// First candidate image that exists on disk.
#[must_use]
pub fn locate_image(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.is_file()).cloned()
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DistroManager;

impl DistroManager {
    pub fn status(&self) -> Result<DistroStatus, WslError> {
        let cli = WslCli;
        let registered = cli
            .list()?
            .iter()
            .any(|d| d.eq_ignore_ascii_case(DISTRO_NAME));
        let running = registered
            && cli
                .running()?
                .iter()
                .any(|d| d.eq_ignore_ascii_case(DISTRO_NAME));
        let install_dir = install_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        Ok(DistroStatus {
            registered,
            running,
            install_dir,
        })
    }

    /// Imports the image into `%LOCALAPPDATA%\Willie\distro`, replacing
    /// a previously registered `willie`. Data migration is a later
    /// slice.
    pub fn install(&self, image: &Path) -> Result<(), WslError> {
        let dir = install_dir().ok_or_else(|| WslError::Unparseable {
            what: "LOCALAPPDATA",
            text: String::new(),
        })?;
        std::fs::create_dir_all(&dir)?;
        self.uninstall()?;
        WslCli.import(DISTRO_NAME, &dir, image)
    }

    pub fn uninstall(&self) -> Result<(), WslError> {
        let cli = WslCli;
        if cli
            .list()?
            .iter()
            .any(|d| d.eq_ignore_ascii_case(DISTRO_NAME))
        {
            let _ = cli.terminate(DISTRO_NAME);
            cli.unregister(DISTRO_NAME)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locate_image_returns_the_first_existing_candidate() {
        let dir = std::env::temp_dir()
            .join(format!("willie-locate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let present = dir.join("willie-rootfs.tar.gz");
        std::fs::write(&present, b"x").unwrap();
        let missing = dir.join("nope.tar.gz");
        assert_eq!(
            locate_image(&[missing.clone(), present.clone()]),
            Some(present)
        );
        assert_eq!(locate_image(&[missing]), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_dir_lives_under_the_willie_data_dir() {
        if let Some(dir) = install_dir() {
            assert!(
                dir.ends_with(r"Willie\distro")
                    || dir.ends_with("Willie/distro")
            );
        }
    }
}
