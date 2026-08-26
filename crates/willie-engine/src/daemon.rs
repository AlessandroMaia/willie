//! Starts, questions and stops `willied` inside the distribution.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use willie_proto::daemon::{DoctorReport, Health, Hello, HelloReply, method};

use crate::{
    error::EngineError, process::WslProcess, rpc::RpcClient, wsl::WslExec,
};

/// Applied only to the first call after a spawn: a cold WSL 2 VM boot
/// was measured at 0.2-0.9s, but can run longer under load.
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
        let mut process = WslProcess::spawn(&WslExec::daemon_stdio())?;
        let transport = process.transport()?;
        let mut client = RpcClient::new(transport, HELLO_TIMEOUT);
        let hello = Hello::for_client("willie-engine");
        let reply: HelloReply = match client.call(method::HELLO, hello) {
            Ok(reply) => reply,
            Err(err) => {
                let stderr = process.stderr_text();
                process.kill();
                let failure = match process.try_wait() {
                    Ok(Some(code)) => EngineError::DaemonExited {
                        code: Some(code),
                        stderr,
                    },
                    _ => err,
                };
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

    /// Records a failed call as the reason the daemon is no longer live,
    /// preserving the original error's code and message.
    fn record(&mut self, err: &EngineError) {
        self.state = DaemonState::Failed {
            code: err.code().to_owned(),
            message: err.to_string(),
        };
        self.live = None;
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
            while std::time::Instant::now() < deadline {
                if process.try_wait()?.is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            process.kill();
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
