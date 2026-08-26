//! `daemon.*` handlers. Each returns the JSON result or a coded error.

use std::time::Instant;

use serde_json::Value;
use willie_proto::{
    PROTOCOL_VERSION,
    daemon::{DoctorReport, Health, Hello, HelloReply},
    rpc::RpcError,
};

fn internal(e: impl std::fmt::Display) -> RpcError {
    RpcError::new("internal_error", e.to_string())
}

pub fn hello(params: Value) -> Result<Value, RpcError> {
    let hello: Hello = serde_json::from_value(params)
        .map_err(|e| RpcError::new("invalid_params", format!("hello: {e}")))?;
    if hello.protocol_version != PROTOCOL_VERSION {
        return Err(RpcError::new(
            "protocol_version_mismatch",
            format!(
                "client speaks protocol {}, daemon speaks {PROTOCOL_VERSION}",
                hello.protocol_version
            ),
        )
        .with_remediation(
            "update Willie so engine and daemon share a version",
        ));
    }
    let reply = HelloReply {
        willie_version: willie_core::VERSION.to_owned(),
        protocol_version: PROTOCOL_VERSION,
        distro_image_version: willie_linux::paths::image_version(),
    };
    serde_json::to_value(reply).map_err(internal)
}

pub fn health(started: Instant) -> Result<Value, RpcError> {
    let health = Health {
        pid: std::process::id(),
        uptime_secs: started.elapsed().as_secs(),
        willie_version: willie_core::VERSION.to_owned(),
    };
    serde_json::to_value(health).map_err(internal)
}

pub fn doctor(run: fn() -> DoctorReport) -> Result<Value, RpcError> {
    serde_json::to_value(run()).map_err(internal)
}
