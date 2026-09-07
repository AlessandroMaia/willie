//! Configuration profiles applied per project.
//!
//! A profile is a versioned directory of fragments (settings, instructions,
//! rules, hooks, MCP servers). Applying one writes into the project and
//! into the harness state while preserving the existing files' format.

use willie_plugin_api::{
    Plugin, PluginCtx, PluginError, PluginManifest, PluginRequest,
    PluginResponse, Scope,
};

/// The profiles plugin.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProfilesPlugin;

/// Constructs the plugin for the daemon's registry.
#[must_use]
pub fn plugin() -> ProfilesPlugin {
    ProfilesPlugin
}

impl Plugin for ProfilesPlugin {
    fn manifest(&self) -> PluginManifest {
        // The id doubles as the method namespace the host routes on:
        // `profile.list`, `profile.create`, … all split to `profile`.
        PluginManifest {
            id: "profile",
            name: "Configuration profiles",
            scope: Scope::PerProject,
        }
    }

    // Placeholder: the profile model (list/create/read_fragment/…) lands
    // in a later task of this slice.
    fn handle(
        &mut self,
        _ctx: &PluginCtx<'_>,
        req: PluginRequest,
    ) -> Result<PluginResponse, PluginError> {
        Err(PluginError::coded(
            "profile_not_implemented",
            format!("profiles has no methods yet (called `{}`)", req.method),
            "the profile model lands in a later task of this slice",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_are_scoped_per_project() {
        assert_eq!(ProfilesPlugin.manifest().scope, Scope::PerProject);
    }
}
