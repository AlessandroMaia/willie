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
//! command calls [`EventPump::ensure`] after it succeeds. If no pump is
//! currently looping, `ensure` takes the engine lock just long enough to
//! ask for a fresh receiver, then hands that receiver to a thread that
//! loops on `recv()` without holding the lock. When `recv()` returns
//! `Err` — the subscription (and with it, almost certainly the daemon)
//! is gone — the thread marks the pump dead and exits, so the next
//! daemon-touching command re-establishes it. One pump, lazily
//! (re)established on demand, is enough for F1a: no reconnect backoff,
//! no replay of events missed while no pump was alive.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use tauri::{AppHandle, Emitter, State};

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
        if self.0.swap(true, Ordering::SeqCst) {
            return; // a pump thread is already looping
        }
        let Some(rx) = state.subscribe_events() else {
            self.0.store(false, Ordering::SeqCst);
            return;
        };
        let app = app.clone();
        let alive = self.0.clone();
        std::thread::spawn(move || {
            while let Ok(notification) = rx.recv() {
                // A closed webview or a serialization hiccup is not the
                // pump's problem to solve: drop the event and keep
                // looping, the next one may go through.
                let _ = app.emit(EVENT, notification.params);
            }
            alive.store(false, Ordering::SeqCst);
        });
    }
}
