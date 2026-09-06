//! Coalescing what the sandbox refuses, so a program that probes one
//! denied call in a loop cannot flood the event log.
//!
//! Pure: the clock is injected and nothing here does I/O. The first
//! refusal of a (class, name) is reported at once; later ones accumulate
//! and are reported as one count when the window has passed since that
//! row last reported, or when the tally is flushed before the session's
//! exit is recorded.

use std::{
    collections::{BTreeMap, btree_map::Entry},
    time::{Duration, Instant},
};

/// One row per (class, name): what has accumulated since the row last
/// reported, and when that was. Rows are never removed — there are as
/// many as the filter has names, a few dozen at most.
#[derive(Debug)]
struct Row {
    pending: u64,
    reported_at: Instant,
}

/// The coalescing tally. See the module doc for the rule it applies.
#[derive(Debug)]
pub struct Tally {
    window: Duration,
    clock: fn() -> Instant,
    rows: BTreeMap<(String, String), Row>,
}

impl Tally {
    #[must_use]
    pub fn new(window: Duration, clock: fn() -> Instant) -> Self {
        Self {
            window,
            clock,
            rows: BTreeMap::new(),
        }
    }

    /// Record one refusal. `Some((class, name, count))` is to be reported
    /// now: the first refusal of this row, or a repeat arriving once the
    /// window has passed since the row last reported, carrying everything
    /// folded since. `None` means the refusal was folded into the row.
    pub fn record(
        &mut self,
        class: &str,
        name: &str,
    ) -> Option<(String, String, u64)> {
        let now = (self.clock)();
        match self.rows.entry((class.to_owned(), name.to_owned())) {
            Entry::Vacant(slot) => {
                let (class, name) = slot.key().clone();
                slot.insert(Row {
                    pending: 0,
                    reported_at: now,
                });
                Some((class, name, 1))
            }
            Entry::Occupied(mut slot) => {
                let row = slot.get_mut();
                row.pending += 1;
                if now.duration_since(row.reported_at) < self.window {
                    return None;
                }
                let count = row.pending;
                row.pending = 0;
                row.reported_at = now;
                let (class, name) = slot.key().clone();
                Some((class, name, count))
            }
        }
    }

    /// Everything still folded, one entry per row in (class, name) order,
    /// each row's window restarting now. For the flush before the
    /// session's exit is recorded, after which nothing is folded in.
    pub fn flush(&mut self) -> Vec<(String, String, u64)> {
        let now = (self.clock)();
        self.rows
            .iter_mut()
            .filter(|(_, row)| row.pending > 0)
            .map(|((class, name), row)| {
                let count = row.pending;
                row.pending = 0;
                row.reported_at = now;
                (class.clone(), name.clone(), count)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        sync::OnceLock,
        time::{Duration, Instant},
    };

    use super::*;

    const WINDOW: Duration = Duration::from_secs(5);

    // Each test runs on its own thread, so a thread-local offset gives
    // every test a clock of its own while `Tally` keeps taking a plain
    // `fn` pointer, the way the event log takes its clock.
    thread_local! {
        static OFFSET_MS: Cell<u64> = const { Cell::new(0) };
    }
    static BASE: OnceLock<Instant> = OnceLock::new();

    fn fake_now() -> Instant {
        *BASE.get_or_init(Instant::now) + Duration::from_millis(OFFSET_MS.get())
    }

    fn advance(ms: u64) {
        OFFSET_MS.set(OFFSET_MS.get() + ms);
    }

    fn fresh() -> Tally {
        OFFSET_MS.set(0);
        Tally::new(WINDOW, fake_now)
    }

    fn row(class: &str, name: &str, count: u64) -> (String, String, u64) {
        (class.to_owned(), name.to_owned(), count)
    }

    #[test]
    fn the_first_denial_of_a_name_is_reported_at_once_with_count_one() {
        let mut tally = fresh();

        assert_eq!(
            tally.record("syscall", "unshare"),
            Some(row("syscall", "unshare", 1))
        );
        assert!(tally.flush().is_empty(), "nothing is left pending");
    }

    #[test]
    fn repeats_inside_the_window_are_folded_and_flushed_as_one_count() {
        let mut tally = fresh();
        tally.record("syscall", "unshare");

        for _ in 0..3 {
            advance(100);
            assert_eq!(tally.record("syscall", "unshare"), None);
        }

        assert_eq!(tally.flush(), vec![row("syscall", "unshare", 3)]);
        assert!(tally.flush().is_empty(), "a flush leaves nothing behind");
    }

    #[test]
    fn a_second_name_and_a_second_class_are_rows_of_their_own() {
        let mut tally = fresh();
        tally.record("syscall", "unshare");

        assert_eq!(
            tally.record("syscall", "setns"),
            Some(row("syscall", "setns", 1))
        );
        assert_eq!(
            tally.record("terminal", "unshare"),
            Some(row("terminal", "unshare", 1))
        );
        assert_eq!(tally.record("syscall", "unshare"), None);
    }

    #[test]
    fn a_flush_reports_every_pending_row_in_a_stable_order() {
        let mut tally = fresh();
        for name in ["unshare", "setns", "unshare", "unshare", "setns"] {
            tally.record("syscall", name);
        }

        assert_eq!(
            tally.flush(),
            vec![row("syscall", "setns", 1), row("syscall", "unshare", 2)]
        );
    }

    #[test]
    fn the_window_boundary_reports_the_accumulated_count() {
        let mut tally = fresh();
        tally.record("syscall", "unshare");
        for _ in 0..3 {
            tally.record("syscall", "unshare");
        }

        advance(4_999);
        assert_eq!(tally.record("syscall", "unshare"), None);
        advance(1);
        assert_eq!(
            tally.record("syscall", "unshare"),
            Some(row("syscall", "unshare", 5)),
            "the four folded repeats and this one, at the window"
        );
        assert_eq!(
            tally.record("syscall", "unshare"),
            None,
            "the window restarts at the report"
        );
    }

    #[test]
    fn a_repeat_after_a_quiet_window_is_reported_alone() {
        let mut tally = fresh();
        tally.record("syscall", "unshare");

        advance(6_000);

        assert_eq!(
            tally.record("syscall", "unshare"),
            Some(row("syscall", "unshare", 1))
        );
    }

    #[test]
    fn a_flush_restarts_the_window_for_the_rows_it_reported() {
        let mut tally = fresh();
        tally.record("syscall", "unshare");
        tally.record("syscall", "unshare");
        advance(4_000);
        assert_eq!(tally.flush(), vec![row("syscall", "unshare", 1)]);

        advance(4_000);
        assert_eq!(
            tally.record("syscall", "unshare"),
            None,
            "four seconds after the flush is inside its window"
        );
        advance(1_000);
        assert_eq!(
            tally.record("syscall", "unshare"),
            Some(row("syscall", "unshare", 2))
        );
    }
}
