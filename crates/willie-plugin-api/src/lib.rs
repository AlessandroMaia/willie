//! Contract between the daemon and its plugins.
//!
//! Plugins are compiled into the daemon. They never run inside a session
//! sandbox, never touch daemon internals, and a failing plugin degrades
//! only itself. The daemon exposes what a plugin needs through a context
//! object that later revisions of this crate will grow.

/// Where a plugin can be enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// One instance for the whole installation.
    Global,
    /// Enabled and configured per project.
    PerProject,
}

/// Static description of a plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginManifest {
    /// Stable identifier used in RPC namespaces, e.g. `usage`.
    pub id: &'static str,
    /// Human-readable name.
    pub name: &'static str,
    /// Where the plugin can be enabled.
    pub scope: Scope,
}

/// A capability hosted by the daemon.
pub trait Plugin: std::fmt::Debug {
    /// Static description.
    fn manifest(&self) -> PluginManifest;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Probe;

    impl Plugin for Probe {
        fn manifest(&self) -> PluginManifest {
            PluginManifest {
                id: "probe",
                name: "Probe",
                scope: Scope::Global,
            }
        }
    }

    #[test]
    fn manifest_is_reachable_through_the_trait_object() {
        let plugin: Box<dyn Plugin> = Box::new(Probe);
        assert_eq!(plugin.manifest().id, "probe");
    }
}
