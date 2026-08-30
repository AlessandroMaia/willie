//! Wire types for the Willie control protocol.
//!
//! Messages are JSON-RPC 2.0 objects, one per line (ndjson). The same
//! messages travel over the engine's stdio pipe to the daemon, over the
//! daemon's Unix socket to local clients and, later, over TCP. This crate
//! only defines the shapes; it performs no I/O.
//!
//! Compatibility rule: every message type ignores unknown fields and gives
//! new fields a default, so an older client can talk to a newer daemon.

pub mod daemon;
pub mod hostterm;
pub mod job;
pub mod project;
pub mod rpc;
pub mod session;
pub mod state;
pub mod supervisor;
pub mod tool;

pub use daemon::{Hello, HelloReply};
pub use rpc::RpcError;

/// Protocol revision. Bumped only for incompatible changes; additive
/// changes keep the number.
pub const PROTOCOL_VERSION: u32 = 1;
