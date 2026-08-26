//! The engine facade the desktop app talks to. Owns the distro manager
//! and the daemon supervisor; every method returns the new status or a
//! typed error so the UI never has to interpret anything.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use willie_proto::daemon::DoctorReport;

use crate::{
    daemon::{DaemonState, DaemonSupervisor},
    distro::{DistroManager, DistroStatus, locate_image},
    error::EngineError,
    prereqs::{WslStatus, wsl_status},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    pub code: String,
    pub message: String,
    pub remediation: String,
}

impl From<&EngineError> for Problem {
    fn from(err: &EngineError) -> Self {
        Self {
            code: err.code().to_owned(),
            message: err.to_string(),
            remediation: err.remediation(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineStatus {
    pub engine_version: String,
    pub wsl: WslStatus,
    pub distro: Option<DistroStatus>,
    pub distro_error: Option<Problem>,
    pub daemon: DaemonState,
    pub doctor: Option<DoctorReport>,
    pub image_available: bool,
}

#[derive(Debug)]
pub struct Engine {
    image_candidates: Vec<PathBuf>,
    distro: DistroManager,
    daemon: DaemonSupervisor,
    last_doctor: Option<DoctorReport>,
}

impl Engine {
    #[must_use]
    pub fn new(image_candidates: Vec<PathBuf>) -> Self {
        Self {
            image_candidates,
            distro: DistroManager,
            daemon: DaemonSupervisor::new(),
            last_doctor: None,
        }
    }

    pub fn status(&mut self) -> EngineStatus {
        let (distro, distro_error) = match self.distro.status() {
            Ok(s) => (Some(s), None),
            Err(e) => (None, Some(Problem::from(&EngineError::Wsl(e)))),
        };
        EngineStatus {
            engine_version: willie_core::VERSION.to_owned(),
            wsl: wsl_status(),
            distro,
            distro_error,
            daemon: self.daemon.state(),
            doctor: self.last_doctor.clone(),
            image_available: locate_image(&self.image_candidates).is_some(),
        }
    }

    pub fn install_distro(&mut self) -> Result<(), EngineError> {
        let image = locate_image(&self.image_candidates)
            .ok_or(EngineError::ImageNotFound)?;
        self.daemon.stop()?;
        self.distro.install(&image)?;
        self.last_doctor = None;
        Ok(())
    }

    /// The engine drives exactly one distribution. Without it `wsl.exe`
    /// answers with its own message and a code the user cannot act on,
    /// so the missing registration is reported before the spawn.
    fn ensure_distro_registered(&self) -> Result<(), EngineError> {
        if self.distro.status()?.registered {
            Ok(())
        } else {
            Err(EngineError::DistroNotRegistered)
        }
    }

    pub fn start_daemon(&mut self) -> Result<(), EngineError> {
        self.ensure_distro_registered()?;
        self.daemon.start().map(drop)
    }

    pub fn stop_daemon(&mut self) -> Result<(), EngineError> {
        self.daemon.stop()
    }

    pub fn run_doctor(&mut self) -> Result<DoctorReport, EngineError> {
        if !matches!(self.daemon.state(), DaemonState::Running { .. }) {
            self.ensure_distro_registered()?;
            self.daemon.start()?;
        }
        let report = match self.daemon.doctor() {
            Ok(report) => report,
            // A well-formed error reply proves the daemon is alive.
            Err(err @ EngineError::Rpc(_)) => return Err(err),
            // Anything else: the supervisor has already reaped a dead or
            // unresponsive daemon. One restart is the supervision
            // promise; a second failure is the user's to see.
            Err(_) => {
                self.ensure_distro_registered()?;
                self.daemon.start()?;
                self.daemon.doctor()?
            }
        };
        self.last_doctor = Some(report.clone());
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problems_carry_code_message_and_remediation() {
        let p = Problem::from(&EngineError::Timeout {
            method: "daemon.health".into(),
        });
        assert_eq!(p.code, "daemon_timeout");
        assert!(p.message.contains("daemon.health"));
        assert!(!p.remediation.is_empty());
    }

    #[test]
    fn status_serialises_with_snake_case_daemon_state() {
        let status = EngineStatus {
            engine_version: "0.1.0".into(),
            wsl: WslStatus {
                installed: false,
                version: None,
                meets_minimum: false,
                minimum: "2.4.4".into(),
            },
            distro: None,
            distro_error: None,
            daemon: DaemonState::Stopped,
            doctor: None,
            image_available: false,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["daemon"]["state"], "stopped");
        assert_eq!(json["wsl"]["minimum"], "2.4.4");
    }
}
