//! Starts, questions and stops `willied` inside the distribution.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use willie_proto::daemon::{DoctorReport, Health, Hello, HelloReply, method};

use crate::{
    error::EngineError, process::WslProcess, rpc::RpcClient, wsl::WslExec,
};

/// The first request pays for the VM boot; allow a minute.
const HELLO_TIMEOUT: Duration = Duration::from_secs(60);
/// Applied to every call once the daemon has already said hello.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_GRACE: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DaemonState {
    #[default]
    Stopped,
    Running {
        willie_version: String,
        image_version: Option<String>,
    },
    Failed {
        code: String,
        message: String,
    },
}

#[derive(Debug, Default)]
pub struct DaemonSupervisor {
    live: Option<(WslProcess, RpcClient)>,
    state: DaemonState,
}

impl DaemonSupervisor {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn state(&self) -> DaemonState {
        self.state.clone()
    }

    pub fn start(&mut self) -> Result<HelloReply, EngineError> {
        self.stop_quietly();
        let mut process = match WslProcess::spawn(&WslExec::daemon_stdio()) {
            Ok(process) => process,
            Err(err) => {
                let failure = EngineError::from(err);
                self.record(&failure);
                return Err(failure);
            }
        };
        let transport = match process.transport() {
            Ok(transport) => transport,
            Err(err) => {
                process.kill();
                let _ = process.wait();
                let failure = EngineError::from(err);
                self.record(&failure);
                return Err(failure);
            }
        };
        let mut client = RpcClient::new(transport, HELLO_TIMEOUT);
        let hello = Hello::for_client("willie-engine");
        let reply: HelloReply = match client.call(method::HELLO, hello) {
            Ok(reply) => reply,
            Err(err) => {
                // Check before killing: a kill always makes `try_wait`
                // report an exit, which would hide the real failure.
                let failure = match process.try_wait() {
                    Ok(Some(code)) => {
                        let stderr = process.stderr_text();
                        EngineError::DaemonExited {
                            code: Some(code),
                            stderr,
                        }
                    }
                    _ => err,
                };
                process.kill();
                let _ = process.wait();
                self.record(&failure);
                return Err(failure);
            }
        };
        if reply.willie_version != willie_core::VERSION {
            let failure = EngineError::VersionMismatch {
                engine: willie_core::VERSION.to_owned(),
                daemon: reply.willie_version.clone(),
            };
            client.close();
            process.kill();
            let _ = process.wait();
            self.record(&failure);
            return Err(failure);
        }
        client.set_timeout(CALL_TIMEOUT);
        self.state = DaemonState::Running {
            willie_version: reply.willie_version.clone(),
            image_version: reply.distro_image_version.clone(),
        };
        self.live = Some((process, client));
        Ok(reply)
    }

    fn client(&mut self) -> Result<&mut RpcClient, EngineError> {
        match &mut self.live {
            Some((_, client)) => Ok(client),
            None => Err(EngineError::Protocol("daemon is not running".into())),
        }
    }

    /// Kills and reaps the process before recording a failure, so an
    /// unresponsive daemon is never left orphaned; preserves the
    /// original error's code and message.
    fn record(&mut self, err: &EngineError) {
        if let Some((mut process, _client)) = self.live.take() {
            process.kill();
            let _ = process.wait();
        }
        self.state = DaemonState::Failed {
            code: err.code().to_owned(),
            message: err.to_string(),
        };
    }

    pub fn health(&mut self) -> Result<Health, EngineError> {
        let result = self.client()?.call(method::HEALTH, serde_json::json!({}));
        if let Err(e) = &result {
            self.record(e);
        }
        result
    }

    pub fn doctor(&mut self) -> Result<DoctorReport, EngineError> {
        let result = self.client()?.call(method::DOCTOR, serde_json::json!({}));
        if let Err(e) = &result {
            self.record(e);
        }
        result
    }

    pub fn stop(&mut self) -> Result<(), EngineError> {
        if let Some((mut process, mut client)) = self.live.take() {
            let _ = client.call::<_, serde_json::Value>(
                method::SHUTDOWN,
                serde_json::json!({}),
            );
            client.close();
            let deadline = std::time::Instant::now() + STOP_GRACE;
            let mut exited = false;
            while std::time::Instant::now() < deadline {
                if matches!(process.try_wait(), Ok(Some(_))) {
                    exited = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            if !exited {
                process.kill();
                let _ = process.wait();
            }
        }
        self.state = DaemonState::Stopped;
        Ok(())
    }

    fn stop_quietly(&mut self) {
        let _ = self.stop();
    }
}

impl Drop for DaemonSupervisor {
    fn drop(&mut self) {
        self.stop_quietly();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_state_serialises_with_the_state_tag() {
        let stopped = serde_json::to_value(DaemonState::Stopped).unwrap();
        assert_eq!(stopped, serde_json::json!({"state": "stopped"}));

        let running = DaemonState::Running {
            willie_version: "0.1.0".into(),
            image_version: None,
        };
        let json = serde_json::to_string(&running).unwrap();
        assert!(json.contains("\"state\":\"running\""));
        assert!(json.contains("\"willie_version\""));

        let failed = DaemonState::Failed {
            code: "daemon_timeout".into(),
            message: "no reply".into(),
        };
        let json = serde_json::to_string(&failed).unwrap();
        assert!(json.contains("\"state\":\"failed\""));
    }
}
