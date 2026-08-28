//! The `tool.*` namespace: managed tools, starting with the harness
//! installer. Replies with the same `JobRef` as project jobs.

use serde::{Deserialize, Serialize};

pub use crate::project::JobRef;

pub mod method {
    pub const INSTALL: &str = "tool.install";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallParams {
    /// Harness id, e.g. `claude-code`.
    pub harness: String,
}
