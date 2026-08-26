//! The Willie distribution as seen from Windows: registered? running?
//! install or remove it from the bundled image.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    error::{EngineError, WslError},
    paths::data_dir,
    wsl::WslCli,
};

pub use crate::wsl::DISTRO_NAME;

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

/// `<image>.sha256`, the sidecar the builder writes next to an image.
fn sidecar_path(image: &Path) -> PathBuf {
    let mut name = image.as_os_str().to_owned();
    name.push(".sha256");
    PathBuf::from(name)
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Rejects a missing or empty image outright; when a `.sha256` sidecar
/// sits next to it, also requires the file's hash to match its first
/// whitespace-separated token. No sidecar means no hash check.
pub fn validate_image(image: &Path) -> Result<(), EngineError> {
    let invalid = |reason: &str| EngineError::ImageInvalid {
        path: image.display().to_string(),
        reason: reason.to_owned(),
    };
    let metadata = std::fs::metadata(image).map_err(|_| invalid("missing"))?;
    if metadata.len() == 0 {
        return Err(invalid("empty"));
    }
    let Some(expected) = std::fs::read_to_string(sidecar_path(image))
        .ok()
        .and_then(|text| text.split_whitespace().next().map(str::to_owned))
    else {
        return Ok(());
    };
    let bytes = std::fs::read(image).map_err(|_| invalid("unreadable"))?;
    if expected.eq_ignore_ascii_case(&hex_sha256(&bytes)) {
        Ok(())
    } else {
        Err(invalid("sha256 mismatch"))
    }
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

    /// Imports the image into `%LOCALAPPDATA%\Willie\data\distro`,
    /// replacing a previously registered `willie`. The image is
    /// validated before the old distribution is touched, so a bad
    /// image never costs the developer their working install. Data
    /// migration is a later slice.
    pub fn install(&self, image: &Path) -> Result<(), EngineError> {
        validate_image(image)?;
        let dir = install_dir().ok_or_else(|| WslError::Unparseable {
            what: "LOCALAPPDATA",
            text: String::new(),
        })?;
        std::fs::create_dir_all(&dir).map_err(WslError::Io)?;
        self.uninstall()?;
        WslCli.import(DISTRO_NAME, &dir, image)?;
        Ok(())
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
                dir.ends_with(r"Willie\data\distro")
                    || dir.ends_with("Willie/data/distro")
            );
        }
    }

    fn temp_image(case: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir()
            .join(format!("willie-validate-{case}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        (dir.clone(), dir.join("image.tar.gz"))
    }

    #[test]
    fn validate_image_accepts_a_file_whose_sidecar_hash_matches() {
        let (dir, image) = temp_image("ok");
        std::fs::write(&image, b"payload").unwrap();
        let hash = hex_sha256(b"payload");
        std::fs::write(sidecar_path(&image), format!("{hash}  x\n")).unwrap();
        assert!(validate_image(&image).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_image_rejects_a_hash_mismatch() {
        let (dir, image) = temp_image("bad-hash");
        std::fs::write(&image, b"payload").unwrap();
        std::fs::write(sidecar_path(&image), "not-the-real-hash\n").unwrap();
        let err = validate_image(&image).unwrap_err();
        assert_eq!(err.code(), "image_invalid");
        assert!(err.to_string().contains("sha256 mismatch"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_image_rejects_an_empty_file() {
        let (dir, image) = temp_image("empty");
        std::fs::write(&image, b"").unwrap();
        let err = validate_image(&image).unwrap_err();
        assert_eq!(err.code(), "image_invalid");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
