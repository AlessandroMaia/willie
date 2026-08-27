//! Helpers that only make sense inside the Willie distribution: well-known
//! paths and the `doctor` checks. Shared by the daemon and the CLI so both
//! report identical results. Compiles everywhere; runs meaningfully on Linux.

#![deny(clippy::unwrap_used, clippy::expect_used)]

pub mod doctor;
pub mod paths;
pub mod wire;
