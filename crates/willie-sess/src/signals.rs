//! `SIGTERM`/`SIGHUP` as a clean shutdown. The signals are blocked in
//! every thread and consumed by one waiter, so they can never interrupt
//! a syscall halfway and always turn into the ordinary stop path.

use std::sync::Arc;

use willie_proto::supervisor::CloseReason;

use crate::server::{self, Shared};

fn shutdown_set() -> libc::sigset_t {
    // SAFETY: `set` is fully initialised by sigemptyset before use.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGHUP);
        set
    }
}

/// Block the shutdown signals in the calling thread. Must run before
/// any other thread is spawned, so they all inherit the mask.
pub fn block_shutdown_signals() {
    let set = shutdown_set();
    // SAFETY: a valid set; the old mask is not needed.
    unsafe {
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut())
    };
}

/// One thread waits for the blocked signals and requests a stop.
pub fn spawn_waiter(shared: Arc<Shared>) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("signals".to_owned())
        .spawn(move || {
            let set = shutdown_set();
            let mut signal: libc::c_int = 0;
            loop {
                // SAFETY: the set is valid and `signal` is an out parameter.
                if unsafe { libc::sigwait(&set, &mut signal) } == 0 {
                    server::request_stop(
                        &shared,
                        "signal",
                        CloseReason::Shutdown,
                    );
                }
            }
        })
        .map(|_| ())
}
