//! Usage, cost and context tracking.
//!
//! Sources are per provider and may be fragile (undocumented endpoints);
//! the plugin caches, marks stale data as such and never fails visibly.

use willie_plugin_api::{
    Plugin, PluginCtx, PluginError, PluginManifest, PluginRequest,
    PluginResponse, Scope,
};

pub mod read;

/// The usage plugin.
#[derive(Debug, Clone, Copy, Default)]
pub struct UsagePlugin;

/// Constructs the plugin for the daemon's registry.
#[must_use]
pub fn plugin() -> UsagePlugin {
    UsagePlugin
}

impl Plugin for UsagePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "usage",
            name: "Usage, cost and context",
            scope: Scope::Global,
        }
    }

    // Registered so the host lists it, but it does nothing until its own
    // slice (F5) gives it real methods.
    fn handle(
        &mut self,
        _ctx: &PluginCtx<'_>,
        req: PluginRequest,
    ) -> Result<PluginResponse, PluginError> {
        Err(PluginError::coded(
            "usage_not_implemented",
            format!("usage has no methods yet (called `{}`)", req.method),
            "usage lands in its own slice; nothing to call yet",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_is_a_global_plugin() {
        assert_eq!(UsagePlugin.manifest().scope, Scope::Global);
    }
}
