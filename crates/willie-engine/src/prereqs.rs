//! What the machine must have before Willie can do anything.

use serde::{Deserialize, Serialize};

use crate::wsl::{WslCli, WslVersion};

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

#[must_use]
pub fn wsl_status() -> WslStatus {
    let minimum = {
        let m = WslVersion::MINIMUM;
        format!("{}.{}.{}", m.major, m.minor, m.patch)
    };
    match WslCli.version() {
        Ok(v) => WslStatus {
            installed: true,
            version: Some(render(v)),
            meets_minimum: v.meets_minimum(),
            minimum,
        },
        Err(_) => WslStatus {
            installed: false,
            version: None,
            meets_minimum: false,
            minimum,
        },
    }
}
