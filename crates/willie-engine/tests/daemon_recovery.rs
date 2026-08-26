//! The supervision promise: a daemon that dies behind the engine's back
//! is noticed and restarted on the next user action.
//!
//! This test needs the `willie` distribution registered and skips with a
//! printed reason otherwise. It boots the VM twice (a few seconds) and
//! terminates the `willie` distribution: a developer with the app open
//! will see its daemon restart.

#![cfg(windows)]

use std::time::{Duration, Instant};

use willie_engine::{
    Engine,
    daemon::DaemonState,
    distro::{DISTRO_NAME, DistroManager},
    wsl::WslCli,
};

/// `true` only when this host can run the test, after printing the one
/// reason it is being skipped.
fn distro_is_registered() -> bool {
    if WslCli.version().is_err() {
        eprintln!("skip: wsl.exe not available");
        return false;
    }
    match DistroManager.status() {
        Ok(status) if status.registered => true,
        Ok(_) => {
            eprintln!("skip: `{DISTRO_NAME}` is not registered");
            false
        }
        Err(err) => {
            eprintln!("skip: cannot read the distribution status: {err}");
            false
        }
    }
}

#[test]
fn run_doctor_restarts_a_daemon_terminated_behind_the_engines_back() {
    if !distro_is_registered() {
        return;
    }
    let mut engine = Engine::new(Vec::new());
    engine.run_doctor().expect("first doctor");

    WslCli
        .terminate(DISTRO_NAME)
        .expect("terminate the distribution");

    // The engine was told nothing; only the child probe can notice.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut observed = engine.status().daemon;
    while matches!(observed, DaemonState::Running { .. })
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(50));
        observed = engine.status().daemon;
    }
    let DaemonState::Failed { code, .. } = &observed else {
        panic!("expected a failed daemon after --terminate, got {observed:?}");
    };
    assert_eq!(code, "daemon_exited");

    engine.run_doctor().expect("doctor after terminate");
    engine.stop_daemon().expect("stop the restarted daemon");
}
