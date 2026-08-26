//! The supervision promise: a daemon that dies behind the engine's back
//! is noticed and restarted on the next user action.
//!
//! This test disturbs the machine: it boots the VM twice (a few seconds)
//! and terminates the `willie` distribution, so a developer with the app
//! open watches its daemon restart. It therefore runs only when
//! `WILLIE_TEST_DISTRO` names that distribution, and skips with a
//! printed reason otherwise:
//!
//! ```text
//! $env:WILLIE_TEST_DISTRO = "willie"
//! cargo test -p willie-engine --locked --test daemon_recovery -- --nocapture
//! ```

#![cfg(windows)]

use std::time::{Duration, Instant};

use willie_engine::{
    Engine,
    daemon::DaemonState,
    distro::{DISTRO_NAME, DistroManager},
    wsl::WslCli,
};

/// `true` only when the user has opted in for exactly the distribution
/// the engine drives; the engine has no other one to be pointed at.
fn opted_in() -> bool {
    match std::env::var("WILLIE_TEST_DISTRO") {
        Ok(name) if name == DISTRO_NAME => true,
        _ => {
            eprintln!(
                "skip: set WILLIE_TEST_DISTRO={DISTRO_NAME} to run the \
                 recovery test (terminates the distribution)"
            );
            false
        }
    }
}

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
    if !opted_in() || !distro_is_registered() {
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
