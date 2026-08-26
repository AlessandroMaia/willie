//! Usage, cost and context tracking.
//!
//! Sources are per provider and may be fragile (undocumented endpoints);
//! the plugin caches, marks stale data as such and never fails visibly.

use willie_plugin_api::{Plugin, PluginManifest, Scope};

/// The usage plugin.
#[derive(Debug, Clone, Copy, Default)]
pub struct UsagePlugin;

impl Plugin for UsagePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "usage",
            name: "Usage, cost and context",
            scope: Scope::Global,
        }
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
