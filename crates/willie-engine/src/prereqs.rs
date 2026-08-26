//! What the machine must have before Willie can do anything.

use serde::{Deserialize, Serialize};

use crate::{
    error::WslError,
    wsl::{WslCli, WslVersion},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WslStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub meets_minimum: bool,
    pub minimum: String,
}

fn render(v: WslVersion) -> String {
    format!("{}.{}.{}.{}", v.major, v.minor, v.patch, v.build)
}

/// A `wsl.exe` that runs but reports no usable version is still
/// installed: the remedy is an update, not an installation. Only a
/// failure to run the binary at all means WSL is absent.
fn status_from(
    version: Result<WslVersion, WslError>,
    minimum: String,
) -> WslStatus {
    match version {
        Ok(v) => WslStatus {
            installed: true,
            version: Some(render(v)),
            meets_minimum: v.meets_minimum(),
            minimum,
        },
        Err(WslError::NotInstalled(_)) => WslStatus {
            installed: false,
            version: None,
            meets_minimum: false,
            minimum,
        },
        Err(_) => WslStatus {
            installed: true,
            version: None,
            meets_minimum: false,
            minimum,
        },
    }
}

#[must_use]
pub fn wsl_status() -> WslStatus {
    let minimum = {
        let m = WslVersion::MINIMUM;
        format!("{}.{}.{}", m.major, m.minor, m.patch)
    };
    status_from(WslCli.version(), minimum)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimum() -> String {
        "2.4.4".to_owned()
    }

    #[test]
    fn a_wsl_exe_that_cannot_be_run_is_not_installed() {
        let missing = WslError::NotInstalled(std::io::Error::other("nope"));
        let status = status_from(Err(missing), minimum());
        assert!(!status.installed);
        assert_eq!(status.version, None);
        assert!(!status.meets_minimum);
    }

    #[test]
    fn a_present_wsl_exe_without_a_version_stays_installed() {
        let unparseable = WslError::Unparseable {
            what: "wsl --version",
            text: "no version here".into(),
        };
        let status = status_from(Err(unparseable), minimum());
        assert!(status.installed);
        assert_eq!(status.version, None);
        assert!(!status.meets_minimum);
    }

    #[test]
    fn a_reported_version_carries_its_build_number() {
        let v = WslVersion {
            major: 2,
            minor: 6,
            patch: 1,
            build: 0,
        };
        let status = status_from(Ok(v), minimum());
        assert!(status.installed);
        assert_eq!(status.version.as_deref(), Some("2.6.1.0"));
        assert!(status.meets_minimum);
    }
}
