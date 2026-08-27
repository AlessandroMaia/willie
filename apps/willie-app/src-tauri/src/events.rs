//! Live event pump: forwards the daemon's notification stream to the
//! webview as `daemon://event`.
//!
//! `Engine::subscribe_events` only yields a receiver while a daemon is
//! live, and the daemon is not running at app startup, so there is
//! nothing to subscribe to yet. Worse, every time a command starts the
//! daemon on demand, the supervisor builds a brand-new `RpcClient` with
//! its own fresh subscriber list — any subscription taken before that
//! point is now watching a channel nothing will ever send on again.
//!
//! Rather than poll or reconnect on a timer, every daemon-touching
//! command calls [`EventPump::ensure`] after it succeeds. The actual
//! decision — is a pump already looping, and if not, does subscribing
//! now actually get us one — is [`establish`], a small Tauri-free
//! function so the race it resolves can be unit-tested without a live
//! app, engine or daemon. `ensure` only supplies the Tauri-specific
//! parts: a `subscribe` closure that takes the engine lock just long
//! enough to ask for a fresh receiver, and a `run` closure
//! ([`spawn_pump`]) that hands the receiver to a named thread looping on
//! `recv()` without holding that lock.

use std::sync::mpsc::Receiver;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use tauri::{AppHandle, Emitter, State};
use willie_proto::rpc::Notification;

use crate::state::EngineState;

/// Event the webview listens on for live daemon state.
pub const EVENT: &str = "daemon://event";

/// Shared "is a pump thread currently looping" flag, cloned into the
/// spawned thread so it can be cleared after the thread exits.
#[derive(Debug, Clone)]
pub struct EventPump(Arc<AtomicBool>);

impl Default for EventPump {
    fn default() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
}

impl EventPump {
    /// Ensures exactly one pump thread is looping on the engine's current
    /// event subscription, establishing one if none is alive. A no-op
    /// when a pump is already running or when the engine has no live
    /// daemon to subscribe to.
    pub fn ensure(&self, app: &AppHandle, state: &State<'_, EngineState>) {
        let app = app.clone();
        establish(
            &self.0,
            || state.subscribe_events(),
            move |rx, alive| {
                spawn_pump(app, rx, alive);
            },
        );
    }
}

/// The pump's guard, pulled out of [`EventPump::ensure`] so the race it
/// resolves is unit-testable in isolation.
///
/// Exactly one caller per "generation" wins the swap and gets to
/// subscribe and hand its receiver to `run`; every other concurrent
/// caller sees `alive` already `true` and is a no-op. This matters
/// because subscribing is not idempotent — the daemon's `RpcClient`
/// registers an independent channel on every call, and every future
/// notification is broadcast to every registered channel — so two
/// winners would mean every event reaching the webview twice.
///
/// A caller that wins the swap but finds nothing to subscribe to
/// (`subscribe` returns `None`, e.g. no daemon is running yet) resets
/// the flag so a later call can retry, rather than leaving the pump
/// permanently marked alive with no thread actually watching anything.
///
/// `establish` never moves `rx` across a thread boundary itself, so `T`
/// carries no `Send`/`'static` bound here; whichever `run` a caller
/// supplies is where that requirement is actually enforced (see
/// [`spawn_pump`]).
fn establish<T>(
    alive: &Arc<AtomicBool>,
    subscribe: impl FnOnce() -> Option<Receiver<T>>,
    run: impl FnOnce(Receiver<T>, Arc<AtomicBool>),
) {
    if alive.swap(true, Ordering::SeqCst) {
        return; // a pump is already looping
    }
    match subscribe() {
        Some(rx) => run(rx, alive.clone()),
        None => alive.store(false, Ordering::SeqCst),
    }
}

/// Spawns the named thread that loops on `recv()` without holding the
/// engine lock, forwarding every notification's `params` (the
/// `willie_proto::state::Event` JSON) as `daemon://event`. When `recv()`
/// returns `Err` — the subscription, and with it almost certainly the
/// daemon, is gone — the thread clears `alive` and exits, so the next
/// `ensure` call re-establishes it.
///
/// A spawn failure is vanishingly unlikely (the host would need to be
/// out of threads) but is not swallowed silently: it clears `alive`
/// too, so the pump is retried by a later command instead of being
/// wedged permanently "alive" with nothing watching.
fn spawn_pump(
    app: AppHandle,
    rx: Receiver<Notification>,
    alive: Arc<AtomicBool>,
) {
    // Cloned so `alive` is still ours to clear here if the spawn itself
    // fails — the closure below owns the only other handle.
    let for_thread = alive.clone();
    let spawned = std::thread::Builder::new()
        .name("willie-event-pump".into())
        .spawn(move || {
            // A closed webview or a serialization hiccup is not the
            // pump's problem to solve: drop the event and keep looping,
            // the next one may go through.
            for notification in rx {
                let _ = app.emit(EVENT, notification.params);
            }
            for_thread.store(false, Ordering::SeqCst);
        });
    if let Err(e) = spawned {
        eprintln!("willie-app: cannot spawn event pump: {e}");
        alive.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Barrier;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn only_one_of_two_racing_calls_subscribes() {
        let alive = Arc::new(AtomicBool::new(false));
        let subscribe_calls = Arc::new(AtomicUsize::new(0));
        let run_calls = Arc::new(AtomicUsize::new(0));
        // Lines both threads up so the swap itself, not scheduling luck,
        // is what decides the single winner.
        let barrier = Arc::new(Barrier::new(2));

        let handles: Vec<_> = (0..2)
            .map(|_| {
                let alive = alive.clone();
                let subscribe_calls = subscribe_calls.clone();
                let run_calls = run_calls.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    establish(
                        &alive,
                        || {
                            subscribe_calls.fetch_add(1, Ordering::SeqCst);
                            let (_tx, rx) = mpsc::channel::<()>();
                            Some(rx)
                        },
                        |_rx, _alive| {
                            run_calls.fetch_add(1, Ordering::SeqCst);
                        },
                    );
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(subscribe_calls.load(Ordering::SeqCst), 1);
        assert_eq!(run_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_no_daemon_call_leaves_the_flag_resettable() {
        let alive = Arc::new(AtomicBool::new(false));

        establish::<()>(
            &alive,
            || None,
            |_rx, _alive| {
                panic!(
                    "run must not fire when there is nothing to subscribe to"
                );
            },
        );
        assert!(!alive.load(Ordering::SeqCst));

        // A later call, once something is actually there to subscribe
        // to, must establish rather than staying stuck.
        let ran = Arc::new(AtomicBool::new(false));
        let ran_signal = ran.clone();
        establish(
            &alive,
            || {
                let (_tx, rx) = mpsc::channel::<()>();
                Some(rx)
            },
            move |_rx, _alive| ran_signal.store(true, Ordering::SeqCst),
        );
        assert!(ran.load(Ordering::SeqCst));
    }

    #[test]
    fn the_flag_resets_when_the_subscription_ends() {
        let alive = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel::<()>();
        drop(tx); // the subscription is already gone before the pump runs

        establish(
            &alive,
            move || Some(rx),
            |rx, alive| {
                // Deterministic stand-in for the real pump thread: drain to
                // `Err` (here, immediately), then clear the flag exactly
                // like the real loop does when it exits.
                for _ in rx {}
                alive.store(false, Ordering::SeqCst);
            },
        );
        assert!(!alive.load(Ordering::SeqCst));

        // A subsequent call must re-establish, not treat the ended
        // subscription as still alive.
        let ran = Arc::new(AtomicBool::new(false));
        let ran_signal = ran.clone();
        establish(
            &alive,
            || {
                let (_tx, rx) = mpsc::channel::<()>();
                Some(rx)
            },
            move |_rx, _alive| ran_signal.store(true, Ordering::SeqCst),
        );
        assert!(ran.load(Ordering::SeqCst));
    }
}
