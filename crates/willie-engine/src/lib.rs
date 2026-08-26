//! Windows-side engine.
//!
//! Provisions the Willie WSL distribution, supervises the daemon that runs
//! inside it, and adapts Windows facilities (proxy, certificates, Windows
//! Terminal, tray) for the rest of the system. Everything that knows
//! Windows exists lives here; the daemon never does.
//!
//! Pure command-building logic is kept platform-independent so it can be
//! unit-tested on any host; only process spawning is Windows-specific.

pub mod paths;
pub mod text;
pub mod wsl;
