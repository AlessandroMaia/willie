//! Background job runner: bounded concurrency, per-project exclusion,
//! cancellation and a clean shutdown that fails still-running jobs.

use std::{
    any::Any,
    collections::{HashMap, HashSet},
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc, Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use willie_core::id::{JobId, ProjectId};
use willie_proto::job::{Job, JobKind, JobState};

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

    pub(crate) fn trip(&self) {
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

/// Releases the pool slot and clears the busy/cancel bookkeeping for one
/// job when its worker thread ends. Built as an RAII guard so cleanup
/// still runs if `work` (or anything else in the thread) unwinds past
/// the point where it is constructed -- a leaked slot would otherwise
/// jam the pool and a leaked busy entry would wedge its project forever.
struct Cleanup {
    pool: Arc<Pool>,
    busy: Arc<Mutex<HashSet<ProjectId>>>,
    cancels: Arc<Mutex<HashMap<JobId, Cancel>>>,
    project_id: Option<ProjectId>,
    id: JobId,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        self.pool.release();
        if let Some(project_id) = self.project_id {
            lock(&self.busy).remove(&project_id);
        }
        lock(&self.cancels).remove(&self.id);
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
        self.spawn_job(kind, Some(project_id), work)
    }

    /// A job that belongs to no project (a tool install). It shares the
    /// pool but not the per-project exclusion.
    // Called from the harness install RPC once it lands; allow until then
    // so the plain (non-test) binary still builds clean.
    #[allow(dead_code)]
    pub fn submit_global(
        &self,
        kind: JobKind,
        work: Work,
    ) -> Result<JobId, ()> {
        self.spawn_job(kind, None, work)
    }

    /// Shared machinery behind [`Runner::submit`] and
    /// [`Runner::submit_global`]: registers the job, emits its running
    /// state, and runs `work` on a pooled thread. The per-project `busy`
    /// entry is the caller's concern -- the [`Cleanup`] guard here only
    /// clears it when the job actually has a project.
    fn spawn_job(
        &self,
        kind: JobKind,
        project_id: Option<ProjectId>,
        work: Work,
    ) -> Result<JobId, ()> {
        let id = JobId::new();
        let cancel = Cancel::default();
        lock(&self.cancels).insert(id, cancel.clone());
        let started_at = (self.clock)();
        let job = Job {
            id,
            kind,
            project_id,
            state: JobState::Running,
            started_at: started_at.clone(),
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
            let cleanup = Cleanup {
                pool,
                busy,
                cancels,
                project_id,
                id,
            };
            let outcome = if cancel.is_cancelled() {
                Err(cancelled_outcome())
            } else {
                match panic::catch_unwind(AssertUnwindSafe(|| work(&cancel))) {
                    Ok(outcome) => outcome,
                    Err(payload) => Err(panic_outcome(payload)),
                }
            };
            drop(cleanup);
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
                started_at,
                finished_at: Some(clock()),
                log_tail: tail(&log),
            };
            crate::state::emit(&state, &out, |s| s.upsert_job(done));
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
        crate::state::emit(&self.state, &self.out, |s| s.upsert_job(job));
    }
}

/// The `(code, message, remediation)` triple for a job whose cancel was
/// tripped before its work ever started.
fn cancelled_outcome() -> (String, String, String) {
    (
        "cancelled".to_owned(),
        "the job was cancelled".to_owned(),
        "start it again if you still need it".to_owned(),
    )
}

/// The `(code, message, remediation)` triple for a job whose `work`
/// closure panicked instead of returning. Recovers the panic message
/// when it is a plain `&str` or `String` (the common case for `panic!`
/// and `.expect(...)`), otherwise falls back to a generic message.
fn panic_outcome(payload: Box<dyn Any + Send>) -> (String, String, String) {
    let message = if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "the job thread panicked".to_owned()
    };
    (
        "job_panicked".to_owned(),
        message,
        "try the operation again; if it keeps happening, report it".to_owned(),
    )
}

fn tail(log: &str) -> String {
    const MAX: usize = 4096;
    if log.len() <= MAX {
        return log.to_owned();
    }
    // The naive `len() - MAX` byte offset can land inside a multi-byte
    // UTF-8 sequence (git logs routinely carry accented paths, commit
    // messages, etc.); snap forward to the next char boundary.
    let mut start = log.len() - MAX;
    while !log.is_char_boundary(start) {
        start += 1;
    }
    log[start..].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Barrier, mpsc};

    fn clock() -> String {
        "t".to_owned()
    }

    fn runner() -> (Runner, Arc<Mutex<State>>) {
        let state = Arc::new(Mutex::new(State::default()));
        // The writer sink is a black hole for this test.
        let (out, _h) = Outbound::spawn(std::io::sink());
        (Runner::new(Arc::clone(&state), out, clock), state)
    }

    /// A state and outbound writer for tests that build their own
    /// [`Runner`] directly, e.g. to assert on the state it shares.
    fn test_runner_state()
    -> (Arc<Mutex<State>>, Outbound, thread::JoinHandle<()>) {
        let state = Arc::new(Mutex::new(State::default()));
        let (out, h) = Outbound::spawn(std::io::sink());
        (state, out, h)
    }

    /// Polls `state` for up to three seconds until `id`'s job leaves
    /// `Running`, then returns. Panics if it never does.
    fn wait_for_job_done(state: &Arc<Mutex<State>>, id: JobId) {
        for _ in 0..300 {
            {
                let s = lock(state);
                if let Some(job) = s.jobs.get(&id)
                    && !matches!(job.state, JobState::Running)
                {
                    return;
                }
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("job should have finished within 3 seconds");
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
    fn a_global_job_has_no_project_and_still_runs_to_done() {
        let (state, out, _h) = test_runner_state();
        let runner = Runner::new(state.clone(), out, clock);
        let id = runner
            .submit_global(
                JobKind::InstallHarness,
                Box::new(|_| Ok("ok".into())),
            )
            .unwrap();
        wait_for_job_done(&state, id);
        let job = state.lock().unwrap().jobs[&id].clone();
        assert_eq!(job.project_id, None);
        assert!(matches!(job.state, JobState::Done));
    }

    #[test]
    fn a_different_project_runs_concurrently() {
        let (runner, state) = runner();
        // Both jobs must reach the barrier before either can return; a
        // serial (limit=1) runner would deadlock here since the second
        // job can never start while the first is still blocked on it.
        let barrier = Arc::new(Barrier::new(2));
        let a_barrier = Arc::clone(&barrier);
        let b_barrier = Arc::clone(&barrier);
        let a = runner
            .submit(
                JobKind::Add,
                ProjectId::new(),
                Box::new(move |_| {
                    a_barrier.wait();
                    Ok("a".into())
                }),
            )
            .unwrap();
        let b = runner
            .submit(
                JobKind::Add,
                ProjectId::new(),
                Box::new(move |_| {
                    b_barrier.wait();
                    Ok("b".into())
                }),
            )
            .unwrap();
        // Wait until both jobs are Done in the state; a regression back
        // to serial execution times out here instead of hanging.
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
        // Occupy every pool slot with a job that signals once it is
        // actually running (i.e. has acquired its slot), so the target
        // job below is provably still queued -- not just probably --
        // when it gets cancelled.
        let (started_tx, started_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Arc::new(Mutex::new(release_rx));
        for _ in 0..3 {
            let started_tx = started_tx.clone();
            let release_rx = Arc::clone(&release_rx);
            let _ = runner.submit(
                JobKind::Add,
                ProjectId::new(),
                Box::new(move |_| {
                    started_tx.send(()).unwrap();
                    let _ = release_rx.lock().unwrap().recv();
                    Ok(String::new())
                }),
            );
        }
        for _ in 0..3 {
            started_rx.recv().unwrap();
        }
        let target = runner
            .submit(JobKind::Add, pid, Box::new(|_| Ok("late".into())))
            .unwrap();
        runner.cancel(&target);
        for _ in 0..3 {
            release_tx.send(()).unwrap();
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

    #[test]
    fn tail_does_not_panic_on_multibyte_utf8_near_the_cutoff() {
        // A repeated string that packs 2- and 3-byte UTF-8 characters
        // so a naive `len() - MAX` byte cutoff can land inside one.
        let log: String = "cafe\u{e9} \u{2713} ".repeat(700);
        assert!(log.len() > 4096, "fixture must exceed the tail cutoff");
        let result = tail(&log);
        assert!(result.len() <= log.len());
        assert!(result.len() <= 4096 + 4, "should stay near the cap");
    }

    #[test]
    fn a_panicking_job_fails_with_job_panicked_and_frees_its_slot() {
        let (runner, state) = runner();
        let pid = ProjectId::new();
        let target = runner
            .submit(JobKind::Add, pid, Box::new(|_| panic!("boom")))
            .unwrap();
        for _ in 0..200 {
            let got = state
                .lock()
                .unwrap()
                .jobs
                .get(&target)
                .map(|j| j.state.clone());
            if let Some(JobState::Failed { code, .. }) = got {
                assert_eq!(code, "job_panicked");
                // The busy project id and pool slot must have been
                // freed too: a fresh submit for the same project must
                // now succeed.
                let retry = runner.submit(
                    JobKind::Add,
                    pid,
                    Box::new(|_| Ok(String::new())),
                );
                assert!(retry.is_ok(), "the project must not stay busy");
                return;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the panicking job should have failed with `job_panicked`");
    }
}
