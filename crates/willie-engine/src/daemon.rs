//! Starts, questions and stops `willied` inside the distribution.

use std::time::{Duration, Instant};

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
/// How long a failed hello waits for the child to finish dying
/// before the failure is classified; an exit is the cause, not the
/// symptom.
const EXIT_SETTLE: Duration = Duration::from_millis(500);

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

    /// Re-checks the child before answering: a daemon that died behind
    /// the engine's back (`wsl --terminate`, `--unregister`, a crash) is
    /// reaped and reported as `daemon_exited` instead of a stale Running.
    #[must_use]
    pub fn state(&mut self) -> DaemonState {
        let exited = match &mut self.live {
            Some((process, _)) => match process.try_wait() {
                Ok(Some(code)) => Some(EngineError::DaemonExited {
                    code: Some(code),
                    detail: process.stderr_text(),
                }),
                _ => None,
            },
            None => None,
        };
        if let Some(failure) = exited {
            self.record(&failure);
        }
        self.state.clone()
    }

    pub fn start(&mut self) -> Result<HelloReply, EngineError> {
        self.start_with(|| WslProcess::spawn(&WslExec::daemon_stdio()))
    }

    /// The whole start sequence over an injected child, so a test can
    /// drive the classification of a failed hello without a WSL
    /// installation. `start` is the only production caller.
    fn start_with(
        &mut self,
        spawn: impl FnOnce() -> Result<WslProcess, crate::error::WslError>,
    ) -> Result<HelloReply, EngineError> {
        self.stop_quietly();
        let mut process = match spawn() {
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
                // Give an exit that caused this failure time to land, so
                // the cause is reported instead of the transport or
                // protocol symptom it produced.
                let deadline = Instant::now() + EXIT_SETTLE;
                while Instant::now() < deadline {
                    if matches!(process.try_wait(), Ok(Some(_))) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                // Check before killing: a kill always makes `try_wait`
                // report an exit, which would hide the real failure.
                let failure = match process.try_wait() {
                    Ok(Some(code)) => {
                        // `wsl.exe` writes its own refusal on the
                        // child's stdout and leaves stderr empty, so the
                        // stray lines are the only account of it.
                        let stderr = process.stderr_text();
                        let detail = if stderr.trim().is_empty() {
                            client.stray_text()
                        } else {
                            stderr
                        };
                        EngineError::DaemonExited {
                            code: Some(code),
                            detail,
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
            None => Err(EngineError::DaemonNotRunning),
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
            // A well-formed RPC error reply proves the daemon is alive;
            // only other failures (timeout, transport, exit) record.
            if !matches!(e, EngineError::Rpc(_)) {
                self.record(e);
            }
        }
        result
    }

    pub fn doctor(&mut self) -> Result<DoctorReport, EngineError> {
        let result = self.client()?.call(method::DOCTOR, serde_json::json!({}));
        if let Err(e) = &result {
            // Same rule as `health`: an RPC error reply is not a dead
            // daemon, so it must not trigger a kill and reap.
            if !matches!(e, EngineError::Rpc(_)) {
                self.record(e);
            }
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
            let deadline = Instant::now() + STOP_GRACE;
            let mut exited = false;
            while Instant::now() < deadline {
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

    /// A daemon can die without the engine calling anything: the VM is
    /// terminated, the distribution is unregistered, `willied` panics.
    /// `state` must notice instead of repeating the last good answer.
    #[cfg(windows)]
    #[test]
    fn state_reports_daemon_exited_once_the_child_is_gone() {
        if std::env::var_os("WILLIE_DIE_MODE").is_some() {
            crate::test_support::announce_ready();
            std::process::exit(3);
        }
        let child = crate::test_support::spawn_peer_with_stderr(
            "daemon::tests::state_reports_daemon_exited_once_the_child_is_gone",
            "WILLIE_DIE_MODE",
        );
        let mut process = WslProcess::from_child(child).unwrap();
        let mut transport = process.transport().unwrap();
        crate::test_support::await_ready(&mut transport);
        let client = RpcClient::new(transport, CALL_TIMEOUT);
        let mut supervisor = DaemonSupervisor {
            live: Some((process, client)),
            state: DaemonState::Running {
                willie_version: "0.1.0".into(),
                image_version: None,
            },
        };

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut observed = supervisor.state();
        while matches!(observed, DaemonState::Running { .. })
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(25));
            observed = supervisor.state();
        }

        let DaemonState::Failed { code, .. } = &observed else {
            panic!("expected a failed daemon, got {observed:?}");
        };
        assert_eq!(code, "daemon_exited");
        assert!(
            supervisor.live.is_none(),
            "the dead child must have been reaped"
        );
    }

    /// `wsl.exe` refusing to start the daemon writes its reason to the
    /// child's stdout as UTF-16LE and nothing to stderr. The failure must
    /// carry that text, not an empty detail.
    #[cfg(windows)]
    #[test]
    fn hello_failure_reports_the_childs_stdout_message_when_stderr_is_empty() {
        if std::env::var_os("WILLIE_UTF16_EXIT_MODE").is_some() {
            crate::test_support::write_utf16_message_and_exit();
        }
        let child = crate::test_support::spawn_peer_with_stderr(
            "daemon::tests::hello_failure_reports_the_childs_stdout\
             _message_when_stderr_is_empty",
            "WILLIE_UTF16_EXIT_MODE",
        );
        let mut supervisor = DaemonSupervisor::new();
        let err = supervisor
            .start_with(|| WslProcess::from_child(child))
            .unwrap_err();

        let EngineError::DaemonExited { code, detail } = &err else {
            panic!("expected DaemonExited, got {err:?}");
        };
        assert_eq!(*code, Some(127));
        assert!(detail.contains("WSL_E_DISTRO_NOT_FOUND"), "{detail}");
        assert!(
            err.remediation().contains("click Install distribution"),
            "{}",
            err.remediation()
        );
        assert!(supervisor.live.is_none(), "the child must be reaped");
    }
}
