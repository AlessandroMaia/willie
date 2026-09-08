//! Managed tools. Today: install the harness, as a project-less job that
//! runs the official installer and re-detects on success. Only ever
//! started by the `tool.install` RPC -- the user's explicit action.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use willie_core::{id::JobId, session::remediation_for};
use willie_harness::Harness;
use willie_proto::{job::JobKind, tool::ToolStatus};

use crate::{
    harness,
    jobs::{JobOutcome, Runner},
    manifest::{self, ToolRecord},
    projects::OpError,
};

/// Start the install job. Refuses when the harness is already present or
/// a tool job is already running.
pub fn install(
    runner: &Runner,
    home: PathBuf,
    state_dir: PathBuf,
    name: &str,
) -> Result<JobId, OpError> {
    if name != harness::claude().id() {
        // Not the shared table's remediation: that one tells the user to
        // reopen the session, and nothing here is about a session.
        return Err(OpError::new(
            "invalid_params",
            "unknown harness",
            "call tool.list for the ids this build can install",
        ));
    }
    if harness::detect_claude(&home).is_some() {
        return Err(OpError::coded(
            "harness_already_installed",
            "Claude Code is already installed",
        ));
    }
    let command = installer_command();
    let id = name.to_owned();
    runner
        .submit_global(
            JobKind::InstallHarness,
            Box::new(move |_cancel| {
                run_installer_and_record(&command, &home, &state_dir, &id)
            }),
        )
        .map_err(|()| {
            OpError::coded("tool_busy", "a tool job is already running")
        })
}

/// Every catalogue tool with live detection merged onto the manifest.
/// The catalogue is the harness registry today.
pub fn list(home: &Path, state_dir: &Path) -> Vec<ToolStatus> {
    let recorded = manifest::load(state_dir);
    willie_harness::registry()
        .iter()
        .map(|h| {
            let version = detect(h.as_ref(), home);
            ToolStatus {
                id: h.id().to_owned(),
                name: display_name(h.id()),
                installed: version.is_some(),
                recorded_version: recorded
                    .get(h.id())
                    .map(|r| r.version.clone()),
                version,
            }
        })
        .collect()
}

/// Re-run the installer for an installed tool, re-detecting and recording
/// the manifest on success. Refuses a tool that is not installed --
/// Update is only offered for one that is, but the daemon fails closed.
pub fn update(
    runner: &Runner,
    home: PathBuf,
    state_dir: PathBuf,
    id: &str,
) -> Result<JobId, OpError> {
    if id != harness::claude().id() {
        return Err(OpError::new(
            "invalid_params",
            "unknown tool",
            "call tool.list for the ids this build manages",
        ));
    }
    if harness::detect_claude(&home).is_none() {
        return Err(OpError::coded(
            "tool_not_installed",
            "the tool is not installed",
        ));
    }
    let command = installer_command();
    let id = id.to_owned();
    runner
        .submit_global(
            JobKind::UpdateHarness,
            Box::new(move |_cancel| {
                run_installer_and_record(&command, &home, &state_dir, &id)
            }),
        )
        .map_err(|()| {
            OpError::coded("tool_busy", "a tool job is already running")
        })
}

/// `WILLIE_HARNESS_INSTALLER` overrides the harness's own installer, so
/// tests can stand in a fake one; production always runs the harness's
/// official command line.
fn installer_command() -> String {
    std::env::var("WILLIE_HARNESS_INSTALLER")
        .unwrap_or_else(|_| harness::claude().installer().to_owned())
}

/// The installed version of one catalogue tool, live. Only Claude Code
/// exists today, so this delegates to `harness::detect_claude`; a second
/// harness moves detection onto a `ManagedTool` trait method each entry
/// in the registry answers for itself.
fn detect(_h: &dyn Harness, home: &Path) -> Option<String> {
    harness::detect_claude(home).map(|i| i.version)
}

/// The catalogue entry whose id is `id`, if the registry has one.
fn find(id: &str) -> Option<Box<dyn Harness>> {
    willie_harness::registry()
        .into_iter()
        .find(|h| h.id() == id)
}

/// The Dashboard's display name for a catalogue tool. A total match with
/// the id itself as the fallback; moves onto a `ManagedTool` trait when a
/// second tool arrives, so the name lives on the tool rather than here.
fn display_name(id: &str) -> String {
    match id {
        "claude-code" => "Claude Code".to_owned(),
        other => other.to_owned(),
    }
}

/// Runs `command` through `sh -c`, inheriting the daemon's environment
/// with `HOME` overridden to `home`. This differs from a session launch,
/// which starts the harness under a closed environment allowlist via
/// `execve`. Success carries stdout as the job's log; a spawn failure or
/// a non-zero exit both fail the job `install_failed`, with the process's
/// stderr (or the spawn error) as the message.
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

/// Runs the installer, then on success re-detects `id` and records the
/// manifest before handing back the job's log. A detection failure right
/// after a successful installer run leaves the job `Ok` regardless -- the
/// install or update itself succeeded; only the manifest entry is
/// missing, and the next `tool.list` falls back to live detection.
fn run_installer_and_record(
    command: &str,
    home: &Path,
    state_dir: &Path,
    id: &str,
) -> JobOutcome {
    let log = run_installer(command, home)?;
    if let Some(version) = find(id).and_then(|h| detect(h.as_ref(), home)) {
        manifest::record(
            state_dir,
            id,
            &ToolRecord {
                version,
                installed_at: crate::real_clock_or_zero(),
                installer: command.to_owned(),
            },
        );
    }
    Ok(log)
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
        let state_dir = scratch("unknown-harness-state");
        let err =
            install(&runner, home.clone(), state_dir.clone(), "not-a-harness")
                .unwrap_err();
        assert_eq!(err.code, "invalid_params");
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn install_refuses_when_the_harness_is_already_present() {
        let (runner, _state) = runner();
        let home = scratch("already-installed");
        plant_claude(&home);
        let state_dir = scratch("already-installed-state");
        let err = install(
            &runner,
            home.clone(),
            state_dir.clone(),
            harness::claude().id(),
        )
        .unwrap_err();
        assert_eq!(err.code, "harness_already_installed");
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn install_runs_the_installer_and_the_job_reaches_done() {
        let (runner, state) = runner();
        let home = scratch("install-ok");
        std::fs::create_dir_all(&home).unwrap();
        let state_dir = scratch("install-ok-state");
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
        let id = install(
            &runner,
            home.clone(),
            state_dir.clone(),
            harness::claude().id(),
        )
        .unwrap();
        wait_for_job_done(&state, id);
        let job = crate::lock(&state).jobs[&id].clone();
        assert!(
            matches!(job.state, willie_proto::job::JobState::Done),
            "job should have reached done: {:?}",
            job.state
        );
        assert!(harness::detect_claude(&home).is_some());
        let recorded = manifest::load(&state_dir);
        assert_eq!(
            recorded
                .get(harness::claude().id())
                .map(|r| r.version.as_str()),
            Some("5.0.0"),
            "install should have recorded the manifest"
        );
        // SAFETY: same single-threaded scope as the set above.
        unsafe {
            std::env::remove_var("WILLIE_HARNESS_INSTALLER");
        }
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn install_refuses_a_second_call_while_one_is_running() {
        let (runner, state) = runner();
        let home = scratch("tool-busy");
        std::fs::create_dir_all(&home).unwrap();
        let state_dir = scratch("tool-busy-state");
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
        let first = install(
            &runner,
            home.clone(),
            state_dir.clone(),
            harness::claude().id(),
        );
        let first_id = first.expect("first install must be accepted");
        let second = install(
            &runner,
            home.clone(),
            state_dir.clone(),
            harness::claude().id(),
        );
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
        let _ = std::fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn list_reports_the_harness_installed_with_its_version() {
        let home = scratch("list-installed");
        plant_claude(&home);
        let state_dir = scratch("list-installed-state");
        let tools = list(&home, &state_dir);
        let claude = tools
            .iter()
            .find(|t| t.id == harness::claude().id())
            .unwrap();
        assert!(claude.installed);
        assert_eq!(claude.version.as_deref(), Some("1.2.3"));
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn list_reports_the_harness_absent_when_not_planted() {
        let home = scratch("list-absent");
        let state_dir = scratch("list-absent-state");
        let claude = list(&home, &state_dir)
            .into_iter()
            .find(|t| t.id == harness::claude().id())
            .unwrap();
        assert!(!claude.installed);
        assert_eq!(claude.version, None);
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn update_refuses_when_the_tool_is_absent() {
        let (runner, _state) = runner();
        let home = scratch("update-absent");
        let state_dir = scratch("update-absent-state");
        let err = update(
            &runner,
            home.clone(),
            state_dir.clone(),
            harness::claude().id(),
        )
        .unwrap_err();
        assert_eq!(err.code, "tool_not_installed");
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn update_runs_the_installer_and_records_the_manifest() {
        let (runner, state) = runner();
        let home = scratch("update-ok");
        std::fs::create_dir_all(&home).unwrap();
        plant_claude(&home);
        let state_dir = scratch("update-ok-state");
        // SAFETY: this test does not run concurrently with another test
        // that reads or writes this process-wide variable.
        unsafe {
            std::env::set_var(
                "WILLIE_HARNESS_INSTALLER",
                format!(
                    "mkdir -p {home}/.local/bin && printf '#!/bin/sh\\necho \
                     9.9.9\\n' > {home}/.local/bin/claude && chmod +x \
                     {home}/.local/bin/claude",
                    home = home.display()
                ),
            );
        }
        let id = update(
            &runner,
            home.clone(),
            state_dir.clone(),
            harness::claude().id(),
        )
        .unwrap();
        wait_for_job_done(&state, id);
        let job = crate::lock(&state).jobs[&id].clone();
        assert!(
            matches!(job.state, willie_proto::job::JobState::Done),
            "job should have reached done: {:?}",
            job.state
        );
        let recorded = manifest::load(&state_dir);
        assert_eq!(
            recorded
                .get(harness::claude().id())
                .map(|r| r.version.as_str()),
            Some("9.9.9"),
            "update should have recorded the manifest"
        );
        // SAFETY: same single-threaded scope as the set above.
        unsafe {
            std::env::remove_var("WILLIE_HARNESS_INSTALLER");
        }
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&state_dir);
    }
}
