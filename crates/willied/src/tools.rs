//! Managed tools. Today: install the harness, as a project-less job that
//! runs the official installer and re-detects on success. Only ever
//! started by the `tool.install` RPC -- the user's explicit action.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use willie_core::{id::JobId, session::remediation_for};
use willie_harness::Harness;
use willie_proto::job::JobKind;

use crate::{
    harness,
    jobs::{JobOutcome, Runner},
    projects::OpError,
};

/// Start the install job. Refuses when the harness is already present or
/// a tool job is already running.
pub fn install(
    runner: &Runner,
    home: PathBuf,
    name: &str,
) -> Result<JobId, OpError> {
    if name != harness::claude().id() {
        return Err(OpError::coded("invalid_params", "unknown harness"));
    }
    if harness::detect_claude(&home).is_some() {
        return Err(OpError::coded(
            "harness_already_installed",
            "Claude Code is already installed",
        ));
    }
    let command = std::env::var("WILLIE_HARNESS_INSTALLER")
        .unwrap_or_else(|_| harness::claude().installer().to_owned());
    runner
        .submit_global(
            JobKind::InstallHarness,
            Box::new(move |_cancel| run_installer(&command, &home)),
        )
        .map_err(|()| {
            OpError::coded("tool_busy", "a tool job is already running")
        })
}

/// Runs `command` through `sh -c` with `HOME` set to `home`, the same way
/// a session's launch environment is built. Success carries stdout as
/// the job's log; a spawn failure or a non-zero exit both fail the job
/// `install_failed`, with the process's stderr (or the spawn error) as
/// the message.
fn run_installer(command: &str, home: &Path) -> JobOutcome {
    let remediation = || remediation_for("install_failed").to_owned();
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .env("HOME", home)
        .output()
        .map_err(|e| {
            ("install_failed".to_owned(), e.to_string(), remediation())
        })?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err((
            "install_failed".to_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            remediation(),
        ))
    }
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::{outbound::Outbound, state::State};

    fn clock() -> String {
        "t".to_owned()
    }

    fn runner() -> (Runner, Arc<Mutex<State>>) {
        let state = Arc::new(Mutex::new(State::default()));
        let (out, _h) = Outbound::spawn(std::io::sink());
        (Runner::new(Arc::clone(&state), out, clock), state)
    }

    /// Polls `state` for up to three seconds until `id`'s job leaves
    /// `Running`. Panics if it never does.
    fn wait_for_job_done(state: &Arc<Mutex<State>>, id: JobId) {
        use willie_proto::job::JobState;
        for _ in 0..300 {
            let left_running = crate::lock(state)
                .jobs
                .get(&id)
                .is_some_and(|j| !matches!(j.state, JobState::Running));
            if left_running {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("job should have finished within 3 seconds");
    }

    /// Plants an executable fake `claude --version` under
    /// `home/.local/bin`, the same layout `harness::detect_claude` looks
    /// for on the session `PATH`.
    fn plant_claude(home: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let bin_dir = home.join(".local/bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let bin = bin_dir.join("claude");
        std::fs::write(&bin, "#!/bin/sh\necho 1.2.3\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("willie-tools-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn install_refuses_an_unknown_harness() {
        let (runner, _state) = runner();
        let home = scratch("unknown-harness");
        let err = install(&runner, home.clone(), "not-a-harness").unwrap_err();
        assert_eq!(err.code, "invalid_params");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn install_refuses_when_the_harness_is_already_present() {
        let (runner, _state) = runner();
        let home = scratch("already-installed");
        plant_claude(&home);
        let err =
            install(&runner, home.clone(), harness::claude().id()).unwrap_err();
        assert_eq!(err.code, "harness_already_installed");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn install_runs_the_installer_and_the_job_reaches_done() {
        let (runner, state) = runner();
        let home = scratch("install-ok");
        std::fs::create_dir_all(&home).unwrap();
        // SAFETY: this test does not run concurrently with another test
        // that reads or writes this process-wide variable.
        unsafe {
            std::env::set_var(
                "WILLIE_HARNESS_INSTALLER",
                format!(
                    "mkdir -p {home}/.local/bin && printf '#!/bin/sh\\necho \
                     5.0.0\\n' > {home}/.local/bin/claude && chmod +x \
                     {home}/.local/bin/claude",
                    home = home.display()
                ),
            );
        }
        let id =
            install(&runner, home.clone(), harness::claude().id()).unwrap();
        wait_for_job_done(&state, id);
        let job = crate::lock(&state).jobs[&id].clone();
        assert!(
            matches!(job.state, willie_proto::job::JobState::Done),
            "job should have reached done: {:?}",
            job.state
        );
        assert!(harness::detect_claude(&home).is_some());
        // SAFETY: same single-threaded scope as the set above.
        unsafe {
            std::env::remove_var("WILLIE_HARNESS_INSTALLER");
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn install_refuses_a_second_call_while_one_is_running() {
        let (runner, state) = runner();
        let home = scratch("tool-busy");
        std::fs::create_dir_all(&home).unwrap();
        // An installer that blocks until the test lets it finish, proving
        // the second `install` call is refused while it is still running.
        let marker = home.join("go");
        // SAFETY: this test does not run concurrently with another test
        // that reads or writes this process-wide variable.
        unsafe {
            std::env::set_var(
                "WILLIE_HARNESS_INSTALLER",
                format!(
                    "while [ ! -f {} ]; do sleep 0.05; done",
                    marker.display()
                ),
            );
        }
        let first = install(&runner, home.clone(), harness::claude().id());
        let first_id = first.expect("first install must be accepted");
        let second = install(&runner, home.clone(), harness::claude().id());
        assert_eq!(second.unwrap_err().code, "tool_busy");
        // Let the first job finish before tearing down its home
        // directory, so no orphaned shell outlives the test.
        std::fs::write(&marker, "").unwrap();
        wait_for_job_done(&state, first_id);
        // SAFETY: same single-threaded scope as the set above.
        unsafe {
            std::env::remove_var("WILLIE_HARNESS_INSTALLER");
        }
        let _ = std::fs::remove_dir_all(&home);
    }
}
