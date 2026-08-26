//! Background job runner: bounded concurrency, per-project exclusion,
//! cancellation and a clean shutdown that fails still-running jobs.
//!
//! Scaffolding until the daemon wires this module in (Task 9).
#![cfg_attr(target_os = "linux", allow(dead_code))]

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use willie_core::id::{JobId, ProjectId};
use willie_proto::{
    job::{Job, JobKind, JobState},
    rpc::Notification,
    state::{Event, method::EVENT},
};

use crate::{outbound::Outbound, state::State};

pub type JobOutcome = Result<String, (String, String, String)>;
pub type Work = Box<dyn FnOnce(&Cancel) -> JobOutcome + Send>;

/// Recovers a poisoned mutex's guard instead of panicking: one thread's
/// panic must not take the whole job runner down with it.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Same recovery for a condvar wait, which returns its own `LockResult`.
fn wait<'a, T>(cv: &Condvar, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    cv.wait(guard).unwrap_or_else(PoisonError::into_inner)
}

/// A cheap, clonable flag a running job polls to notice cancellation.
#[derive(Clone, Default, Debug)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    fn trip(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// A counting semaphore capping how many jobs run at once. A worker that
/// cannot start immediately waits here; it stays submitted regardless.
struct Pool {
    running: Mutex<usize>,
    cv: Condvar,
    limit: usize,
}

impl Pool {
    fn acquire(&self) {
        let mut n = lock(&self.running);
        while *n >= self.limit {
            n = wait(&self.cv, n);
        }
        *n += 1;
    }

    fn release(&self) {
        *lock(&self.running) -= 1;
        self.cv.notify_one();
    }
}

/// Owns job lifecycle, events and per-project exclusion. The `work`
/// closure passed to [`Runner::submit`] does the actual git operation.
pub struct Runner {
    state: Arc<Mutex<State>>,
    out: Outbound,
    clock: fn() -> String,
    pool: Arc<Pool>,
    busy: Arc<Mutex<HashSet<ProjectId>>>,
    cancels: Arc<Mutex<HashMap<JobId, Cancel>>>,
}

impl std::fmt::Debug for Runner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runner").finish_non_exhaustive()
    }
}

impl Runner {
    /// `clock` injects `started_at`/`finished_at` so tests are
    /// deterministic; production passes a real timestamp function.
    #[must_use]
    pub fn new(
        state: Arc<Mutex<State>>,
        out: Outbound,
        clock: fn() -> String,
    ) -> Self {
        Self {
            state,
            out,
            clock,
            pool: Arc::new(Pool {
                running: Mutex::new(0),
                cv: Condvar::new(),
                limit: 3,
            }),
            busy: Arc::new(Mutex::new(HashSet::new())),
            cancels: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Starts `work` for `project_id` on its own thread, waiting for a
    /// free pool slot if the cap is already reached. Refuses a second
    /// job while one is already running for the same project, without
    /// touching that running job.
    pub fn submit(
        &self,
        kind: JobKind,
        project_id: ProjectId,
        work: Work,
    ) -> Result<JobId, ()> {
        {
            let mut busy = lock(&self.busy);
            if !busy.insert(project_id) {
                return Err(());
            }
        }
        let id = JobId::new();
        let cancel = Cancel::default();
        lock(&self.cancels).insert(id, cancel.clone());
        let job = Job {
            id,
            kind,
            project_id,
            state: JobState::Running,
            started_at: (self.clock)(),
            finished_at: None,
            log_tail: String::new(),
        };
        self.emit_job(job);

        let state = Arc::clone(&self.state);
        let out = self.out.clone();
        let pool = Arc::clone(&self.pool);
        let busy = Arc::clone(&self.busy);
        let cancels = Arc::clone(&self.cancels);
        let clock = self.clock;
        thread::spawn(move || {
            pool.acquire();
            let outcome = if cancel.is_cancelled() {
                Err((
                    "cancelled".to_owned(),
                    "the job was cancelled".to_owned(),
                    "start it again if you still need it".to_owned(),
                ))
            } else {
                work(&cancel)
            };
            pool.release();
            lock(&busy).remove(&project_id);
            lock(&cancels).remove(&id);
            let (final_state, log) = match outcome {
                Ok(log) => (JobState::Done, log),
                Err((code, message, remediation)) => (
                    JobState::Failed {
                        code,
                        message,
                        remediation,
                    },
                    String::new(),
                ),
            };
            let done = Job {
                id,
                kind,
                project_id,
                state: final_state,
                started_at: clock(),
                finished_at: Some(clock()),
                log_tail: tail(&log),
            };
            let event = lock(&state).upsert_job(done);
            out.send_notification(event_notification(event));
        });
        Ok(id)
    }

    /// Trips `id`'s cancel flag. A no-op once the job has finished.
    pub fn cancel(&self, id: &JobId) {
        if let Some(c) = lock(&self.cancels).get(id) {
            c.trip();
        }
    }

    /// Trips every still-registered job's cancel, e.g. on daemon
    /// shutdown.
    pub fn shutdown(&self) {
        for c in lock(&self.cancels).values() {
            c.trip();
        }
    }

    fn emit_job(&self, job: Job) {
        let event = lock(&self.state).upsert_job(job);
        self.out.send_notification(event_notification(event));
    }
}

fn tail(log: &str) -> String {
    const MAX: usize = 4096;
    if log.len() <= MAX {
        log.to_owned()
    } else {
        log[log.len() - MAX..].to_owned()
    }
}

fn event_notification(event: Event) -> Notification {
    let params = serde_json::to_value(event).unwrap_or(serde_json::Value::Null);
    Notification::new(EVENT, params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn clock() -> String {
        "t".to_owned()
    }

    fn runner() -> (Runner, Arc<Mutex<State>>) {
        let state = Arc::new(Mutex::new(State::default()));
        // The writer sink is a black hole for this test.
        let (out, _h) = Outbound::spawn(std::io::sink());
        (Runner::new(Arc::clone(&state), out, clock), state)
    }

    #[test]
    fn a_second_job_on_a_busy_project_is_refused() {
        let (runner, _state) = runner();
        let pid = ProjectId::new();
        let (tx, rx) = mpsc::channel::<()>();
        // First job blocks until we let it finish.
        let gate = rx;
        let _ = runner.submit(
            JobKind::Add,
            pid,
            Box::new(move |_| {
                let _ = gate.recv();
                Ok(String::new())
            }),
        );
        let second = runner.submit(
            JobKind::SyncToWindows,
            pid,
            Box::new(|_| Ok(String::new())),
        );
        assert!(second.is_err(), "same project must be busy");
        tx.send(()).unwrap();
    }

    #[test]
    fn a_different_project_runs_concurrently() {
        let (runner, state) = runner();
        let a = runner
            .submit(
                JobKind::Add,
                ProjectId::new(),
                Box::new(|_| Ok("a".into())),
            )
            .unwrap();
        let b = runner
            .submit(
                JobKind::Add,
                ProjectId::new(),
                Box::new(|_| Ok("b".into())),
            )
            .unwrap();
        // Wait until both jobs are Done in the state.
        for _ in 0..200 {
            {
                let s = state.lock().unwrap();
                let done = [a, b].iter().all(|id| {
                    matches!(
                        s.jobs.get(id).map(|j| &j.state),
                        Some(JobState::Done)
                    )
                });
                if done {
                    return;
                }
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("both jobs should have finished");
    }

    #[test]
    fn cancel_before_start_fails_the_job_cancelled() {
        let (runner, state) = runner();
        let pid = ProjectId::new();
        // Occupy the pool with three slow jobs on other projects so the
        // target job is still queued when we cancel it.
        let (tx, rx) = mpsc::channel::<()>();
        let rx = Arc::new(Mutex::new(rx));
        for _ in 0..3 {
            let rx = Arc::clone(&rx);
            let _ = runner.submit(
                JobKind::Add,
                ProjectId::new(),
                Box::new(move |_| {
                    let _ = rx.lock().unwrap().recv();
                    Ok(String::new())
                }),
            );
        }
        let target = runner
            .submit(JobKind::Add, pid, Box::new(|_| Ok("late".into())))
            .unwrap();
        runner.cancel(&target);
        for _ in 0..3 {
            tx.send(()).unwrap();
        }
        for _ in 0..200 {
            let got = state
                .lock()
                .unwrap()
                .jobs
                .get(&target)
                .map(|j| j.state.clone());
            if let Some(JobState::Failed { code, .. }) = got {
                assert_eq!(code, "cancelled");
                return;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the cancelled job should have failed with `cancelled`");
    }
}
