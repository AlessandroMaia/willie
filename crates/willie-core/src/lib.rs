//! Domain types shared by every Willie component.
//!
//! This crate performs no I/O. Anything that touches the filesystem, the
//! network or a process belongs in `willie-engine` (Windows side) or in
//! `willied` / `willie-sess` (Linux side).

pub mod id;
pub mod paths;
pub mod project;
pub mod sandbox;
pub mod session;

/// Version of the Willie workspace this crate was built from.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_semver_shaped() {
        let parts: Vec<&str> = VERSION.split('.').collect();
        assert_eq!(parts.len(), 3, "expected MAJOR.MINOR.PATCH, got {VERSION}");
        for part in parts {
            assert!(
                part.parse::<u32>().is_ok(),
                "non-numeric part in {VERSION}"
            );
        }
    }
}
