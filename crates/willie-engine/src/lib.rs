//! Windows-side engine.
//!
//! Provisions the Willie WSL distribution, supervises the daemon that runs
//! inside it, and adapts Windows facilities (proxy, certificates, Windows
//! Terminal, tray) for the rest of the system. Everything that knows
//! Windows exists lives here; the daemon never does.
//!
//! Pure command-building logic is kept platform-independent so it can be
//! unit-tested on any host; only process spawning is Windows-specific.

pub mod config;
pub mod daemon;
pub mod discover;
pub mod distro;
pub mod embed;
pub mod engine;
pub mod error;
pub mod identity;
pub mod paths;
pub mod prereqs;
pub mod process;
pub mod rpc;
pub mod terminal;
#[cfg(test)]
pub(crate) mod test_support;
pub mod text;
pub mod wsl;

pub use engine::{Engine, EngineStatus, Problem, SessionOpened};
