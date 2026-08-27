//! The six project operations. Fast validation on the calling thread;
//! the git work runs in a job. Every path handed to `git` is a Linux
//! path (`windows_to_drvfs` maps the registered `C:\` source first).
#![cfg_attr(target_os = "linux", allow(dead_code))]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use willie_core::{
    id::ProjectId,
    paths::windows_to_drvfs,
    project::{Project, ProjectState, slug_for, source_key},
};
use willie_proto::{
    job::JobKind,
    project::{AddParams, AddResult, JobRef, RelocateParams, RenameParams},
};

use crate::{
    git::{self, GitError},
    jobs::{Cancel, JobOutcome, Runner, Work},
    state::State,
    store,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpError {
    pub code: &'static str,
    pub message: String,
    pub remediation: String,
}

impl OpError {
    fn new(
        code: &'static str,
        message: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            remediation: remediation.into(),
        }
    }
}

#[derive(Debug)]
pub struct Ops {
    state: Arc<Mutex<State>>,
    runner: Runner,
    state_dir: PathBuf,
    workspaces_dir: PathBuf,
    clock: fn() -> String,
}

/// The Linux path git should use for a registered source. A real source
/// is a Windows path; tests pass a Linux path straight through.
fn source_to_linux(source: &str) -> Option<String> {
    if source.len() >= 2 && source.as_bytes()[1] == b':' {
        windows_to_drvfs(source)
    } else if source.starts_with('/') {
        Some(source.to_owned())
    } else {
        None
    }
}

/// Recovers a poisoned mutex's guard instead of panicking: one job's
/// panic must not take the whole daemon's state access down with it.
fn lock(m: &Mutex<State>) -> MutexGuard<'_, State> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The last path component of a Windows path, without relying on
/// `Path`'s separator (this runs on Linux, where `\` is not special).
fn folder_name(windows_path: &str) -> String {
    windows_path
        .trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(windows_path)
        .to_owned()
}

/// Remediation text for a `git`-level failure code. `source_detached_head`
/// gets a specific hint; everything else (chiefly `git_failed`) falls
/// back to a generic one.
fn remediation_for(code: &str) -> &'static str {
    match code {
        "source_detached_head" => {
            "check out a branch in the Windows checkout, then add again"
        }
        _ => "check git's output and try again",
    }
}

/// Converts a `GitError` from a *fast validation* step into an `OpError`.
fn git_err_to_op(e: GitError) -> OpError {
    OpError::new(e.code, e.message, remediation_for(e.code))
}

/// Converts a `GitError` from inside a job into the job's outcome tuple.
/// Does not touch the project's own state: an ordinary git failure
/// during `sync`/`update`/`relocate` leaves the project exactly as
/// usable as it was before the attempt.
fn git_err_outcome(e: GitError) -> (String, String, String) {
    let remediation = remediation_for(e.code).to_owned();
    (e.code.to_owned(), e.message, remediation)
}

/// Builds a job outcome tuple for a hand-crafted refusal (not a git
/// error). Same "does not touch project state" rule as
/// [`git_err_outcome`].
fn refuse(
    code: &'static str,
    message: impl Into<String>,
    remediation: impl Into<String>,
) -> (String, String, String) {
    (code.to_owned(), message.into(), remediation.into())
}

/// Persists an edited copy of `project` and records the change in
/// state. Best-effort on disk: a write failure here is surfaced the
/// next time the project is read, not by panicking a job thread.
fn update_project(
    state: &Mutex<State>,
    state_dir: &Path,
    mut project: Project,
    edit: impl FnOnce(&mut Project),
) {
    edit(&mut project);
    let _ = store::save(state_dir, &project);
    let _ = lock(state).upsert_project(project);
}

fn mark_ready(state: &Mutex<State>, state_dir: &Path, project: Project) {
    update_project(state, state_dir, project, |p| {
        p.state = ProjectState::Ready;
    });
}

/// Marks `project` `Failed` with a git error's code/message and returns
/// the matching job outcome. Only `add` uses this: a project that never
/// finished being created has no other valid state to fall back to.
fn add_fail(
    state: &Mutex<State>,
    state_dir: &Path,
    project: &Project,
    e: GitError,
) -> (String, String, String) {
    let code = e.code;
    let message = e.message;
    let remediation = remediation_for(code).to_owned();
    update_project(state, state_dir, project.clone(), |p| {
        p.state = ProjectState::Failed {
            code: code.to_owned(),
            message: message.clone(),
            remediation: remediation.clone(),
        };
    });
    (code.to_owned(), message, remediation)
}

/// Checks whether `cancel` has been tripped since the job started
/// running. If so, marks the project `Failed { code: "interrupted" }`
/// (the job runner only trips the flag and ends the *job* `cancelled`
/// before work starts; once work is under way, the project's own state
/// is this module's responsibility) and returns the matching outcome.
fn check_cancelled(
    cancel: &Cancel,
    state: &Mutex<State>,
    state_dir: &Path,
    project: &Project,
) -> Option<(String, String, String)> {
    if !cancel.is_cancelled() {
        return None;
    }
    let message = "the job was interrupted before it finished".to_owned();
    let remediation = "retry the operation".to_owned();
    update_project(state, state_dir, project.clone(), |p| {
        p.state = ProjectState::Failed {
            code: "interrupted".to_owned(),
            message: message.clone(),
            remediation: remediation.clone(),
        };
    });
    Some(("interrupted".to_owned(), message, remediation))
}

fn busy_err() -> OpError {
    OpError::new(
        "project_busy",
        "a job is already running for this project",
        "wait for the current job to finish or cancel it",
    )
}

fn not_found_err(id: ProjectId) -> OpError {
    OpError::new(
        "project_not_found",
        format!("no project with id `{id}`"),
        "check the project id and try again",
    )
}

fn path_not_windows_err(windows_path: &str) -> OpError {
    OpError::new(
        "path_not_windows",
        format!("`{windows_path}` is not a path on a Windows drive"),
        "register a path on a Windows drive",
    )
}

fn not_a_git_repository_err(windows_path: &str) -> OpError {
    OpError::new(
        "not_a_git_repository",
        format!("`{windows_path}` is not a git repository"),
        "run `git init` and a first commit in the folder, then add again",
    )
}

/// `add`'s job: clone the source into the workspace, rename `origin` to
/// `windows`, copy the source's other remotes, disable line-ending
/// translation in the clone and enable `updateInstead` pushes into the
/// source. On success the project becomes `Ready`; on any git failure
/// it becomes `Failed { code, .. }` with that failure's code.
fn run_add(
    cancel: &Cancel,
    state: &Mutex<State>,
    state_dir: &Path,
    src_linux: &str,
    workspace: &Path,
    project: Project,
) -> JobOutcome {
    if let Some(err) = check_cancelled(cancel, state, state_dir, &project) {
        return Err(err);
    }
    let src = Path::new(src_linux);
    let clone_out = git::clone(src, workspace)
        .map_err(|e| add_fail(state, state_dir, &project, e))?;
    if let Some(err) = check_cancelled(cancel, state, state_dir, &project) {
        return Err(err);
    }
    git::run(workspace, &["remote", "rename", "origin", "windows"])
        .map_err(|e| add_fail(state, state_dir, &project, e))?;
    let remotes = git::run(src, &["remote"])
        .map_err(|e| add_fail(state, state_dir, &project, e))?;
    for name in remotes
        .lines()
        .map(str::trim)
        .filter(|n| !n.is_empty() && *n != "origin")
    {
        let url = git::run(src, &["remote", "get-url", name])
            .map_err(|e| add_fail(state, state_dir, &project, e))?;
        git::run(workspace, &["remote", "add", name, url.trim()])
            .map_err(|e| add_fail(state, state_dir, &project, e))?;
    }
    if let Some(err) = check_cancelled(cancel, state, state_dir, &project) {
        return Err(err);
    }
    git::run(workspace, &["config", "core.autocrlf", "false"])
        .map_err(|e| add_fail(state, state_dir, &project, e))?;
    git::run(
        src,
        &["config", "receive.denyCurrentBranch", "updateInstead"],
    )
    .map_err(|e| add_fail(state, state_dir, &project, e))?;
    if let Some(err) = check_cancelled(cancel, state, state_dir, &project) {
        return Err(err);
    }
    mark_ready(state, state_dir, project);
    Ok(clone_out.log)
}

/// `sync_to_windows`'s job: refuse a missing, dirty or branch-mismatched
/// Windows checkout without touching it, else push the workspace's
/// branch into it (accepted by the `updateInstead` config `add` set).
fn run_sync(
    cancel: &Cancel,
    state: &Mutex<State>,
    state_dir: &Path,
    src_linux: &str,
    workspace: &Path,
    branch: &str,
    project: Project,
) -> JobOutcome {
    if let Some(err) = check_cancelled(cancel, state, state_dir, &project) {
        return Err(err);
    }
    let src = Path::new(src_linux);
    if !src.exists() {
        return Err(refuse(
            "source_missing",
            "the Windows checkout is missing",
            "relocate the project to a checkout that still exists",
        ));
    }
    let clean = git::is_clean(src).map_err(git_err_outcome)?;
    if !clean {
        return Err(refuse(
            "windows_tree_dirty",
            "the Windows checkout has uncommitted changes",
            "commit or discard the changes in the Windows checkout, \
             then try again",
        ));
    }
    let current = git::run(src, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map_err(git_err_outcome)?;
    if current.trim() != branch {
        return Err(refuse(
            "windows_branch_mismatch",
            format!(
                "the Windows checkout is on `{}`, the project's branch \
                 is `{branch}`",
                current.trim()
            ),
            "check out the project's branch in the Windows checkout, \
             then try again",
        ));
    }
    if let Some(err) = check_cancelled(cancel, state, state_dir, &project) {
        return Err(err);
    }
    let refspec = format!("HEAD:{branch}");
    git::run(workspace, &["push", "windows", &refspec]).map_err(git_err_outcome)
}

/// `update_from_windows`'s job: fetch the source and fast-forward only;
/// a workspace with commits the source lacks is refused, not merged.
fn run_update(
    cancel: &Cancel,
    state: &Mutex<State>,
    state_dir: &Path,
    src_linux: &str,
    workspace: &Path,
    branch: &str,
    project: Project,
) -> JobOutcome {
    if let Some(err) = check_cancelled(cancel, state, state_dir, &project) {
        return Err(err);
    }
    if !Path::new(src_linux).exists() {
        return Err(refuse(
            "source_missing",
            "the Windows checkout is missing",
            "relocate the project to a checkout that still exists",
        ));
    }
    git::run(workspace, &["fetch", "windows"]).map_err(git_err_outcome)?;
    if let Some(err) = check_cancelled(cancel, state, state_dir, &project) {
        return Err(err);
    }
    let ff_ref = format!("windows/{branch}");
    if git::run(workspace, &["merge-base", "--is-ancestor", "HEAD", &ff_ref])
        .is_err()
    {
        return Err(refuse(
            "workspace_diverged",
            "the workspace has commits the Windows checkout does not",
            "the workspace has commits Windows does not; send them to \
             Windows first",
        ));
    }
    git::run(workspace, &["merge", "--ff-only", &ff_ref])
        .map_err(git_err_outcome)
}

/// `remove`'s job: optionally delete the ext4 workspace (refused when
/// it holds uncommitted changes and `force` is false), then always
/// forget the project. Never touches the Windows checkout.
fn run_remove(
    cancel: &Cancel,
    state: &Mutex<State>,
    state_dir: &Path,
    workspace: &Path,
    delete_workspace: bool,
    force: bool,
    project: Project,
) -> JobOutcome {
    if let Some(err) = check_cancelled(cancel, state, state_dir, &project) {
        return Err(err);
    }
    if delete_workspace && !force {
        let clean = git::is_clean(workspace).map_err(git_err_outcome)?;
        if !clean {
            return Err(refuse(
                "workspace_dirty",
                "the workspace has uncommitted changes",
                "commit or discard the workspace's changes, or remove \
                 with force",
            ));
        }
    }
    if let Some(err) = check_cancelled(cancel, state, state_dir, &project) {
        return Err(err);
    }
    if delete_workspace {
        fs::remove_dir_all(workspace).map_err(|e| {
            refuse(
                "git_failed",
                e.to_string(),
                "check the daemon's permissions on the workspace and \
                 try again",
            )
        })?;
    }
    let _ = store::delete(state_dir, &project.id);
    let _ = lock(state).remove_project(&project.id);
    Ok(String::new())
}

/// `relocate`'s job: refuse a new source whose history does not contain
/// the workspace's current commit, else point the `windows` remote and
/// the stored source at it.
fn run_relocate(
    cancel: &Cancel,
    state: &Mutex<State>,
    state_dir: &Path,
    workspace: &Path,
    new_src_linux: &str,
    new_windows_path: &str,
    project: Project,
) -> JobOutcome {
    if let Some(err) = check_cancelled(cancel, state, state_dir, &project) {
        return Err(err);
    }
    let new_src = Path::new(new_src_linux);
    let head =
        git::run(workspace, &["rev-parse", "HEAD"]).map_err(git_err_outcome)?;
    if !git::head_contains(new_src, head.trim()) {
        return Err(refuse(
            "source_unrelated",
            "the new checkout's history does not contain the \
             workspace's current commit",
            "point relocate at a checkout that shares history with the \
             workspace",
        ));
    }
    if let Some(err) = check_cancelled(cancel, state, state_dir, &project) {
        return Err(err);
    }
    git::run(workspace, &["remote", "set-url", "windows", new_src_linux])
        .map_err(git_err_outcome)?;
    update_project(state, state_dir, project, |p| {
        p.source = new_windows_path.to_owned();
        p.source_present = true;
    });
    Ok(String::new())
}

impl Ops {
    #[must_use]
    pub fn new(
        state: Arc<Mutex<State>>,
        runner: Runner,
        state_dir: PathBuf,
        workspaces_dir: PathBuf,
        clock: fn() -> String,
    ) -> Self {
        Self {
            state,
            runner,
            state_dir,
            workspaces_dir,
            clock,
        }
    }

    fn get_project(&self, id: ProjectId) -> Result<Project, OpError> {
        lock(&self.state)
            .projects
            .get(&id)
            .cloned()
            .ok_or_else(|| not_found_err(id))
    }

    pub fn add(&self, params: AddParams) -> Result<AddResult, OpError> {
        let AddParams { windows_path, name } = params;
        let src_linux = source_to_linux(&windows_path)
            .ok_or_else(|| path_not_windows_err(&windows_path))?;
        let src_path = Path::new(&src_linux);
        if !git::is_repo(src_path) {
            return Err(not_a_git_repository_err(&windows_path));
        }
        let key = source_key(&windows_path);
        {
            let state = lock(&self.state);
            if state
                .projects
                .values()
                .any(|p| source_key(&p.source) == key)
            {
                return Err(OpError::new(
                    "project_exists",
                    format!("`{windows_path}` is already registered"),
                    "this checkout is already registered as a project",
                ));
            }
        }
        let branch = git::current_branch(src_path).map_err(git_err_to_op)?;
        let taken: Vec<String> = {
            let state = lock(&self.state);
            state.projects.values().map(|p| p.slug.clone()).collect()
        };
        let folder = folder_name(&windows_path);
        let slug = slug_for(&folder, &taken);
        let _ = fs::create_dir_all(&self.workspaces_dir);
        let workspace = self.workspaces_dir.join(&slug);
        if workspace.exists() {
            return Err(OpError::new(
                "workspace_exists",
                format!("workspace `{}` already exists", workspace.display()),
                "remove the existing workspace directory or rename the \
                 project",
            ));
        }
        let id = ProjectId::new();
        let project = Project {
            id,
            name: name.unwrap_or_else(|| folder.clone()),
            slug,
            source: windows_path,
            workspace: workspace.to_string_lossy().into_owned(),
            branch,
            state: ProjectState::Preparing,
            source_present: true,
            created_at: (self.clock)(),
        };
        let _ = store::save(&self.state_dir, &project);
        let _ = lock(&self.state).upsert_project(project.clone());

        let state = Arc::clone(&self.state);
        let state_dir = self.state_dir.clone();
        let job_project = project.clone();
        let work: Work = Box::new(move |cancel| {
            run_add(
                cancel,
                &state,
                &state_dir,
                &src_linux,
                &workspace,
                job_project,
            )
        });
        let job_id = self
            .runner
            .submit(JobKind::Add, id, work)
            .map_err(|()| busy_err())?;
        Ok(AddResult {
            project_id: id,
            job_id,
        })
    }

    pub fn sync_to_windows(&self, id: ProjectId) -> Result<JobRef, OpError> {
        let project = self.get_project(id)?;
        let src_linux = source_to_linux(&project.source)
            .ok_or_else(|| path_not_windows_err(&project.source))?;
        let state = Arc::clone(&self.state);
        let state_dir = self.state_dir.clone();
        let workspace = PathBuf::from(project.workspace.clone());
        let branch = project.branch.clone();
        let job_project = project;
        let work: Work = Box::new(move |cancel| {
            run_sync(
                cancel,
                &state,
                &state_dir,
                &src_linux,
                &workspace,
                &branch,
                job_project,
            )
        });
        let job_id = self
            .runner
            .submit(JobKind::SyncToWindows, id, work)
            .map_err(|()| busy_err())?;
        Ok(JobRef { job_id })
    }

    pub fn update_from_windows(
        &self,
        id: ProjectId,
    ) -> Result<JobRef, OpError> {
        let project = self.get_project(id)?;
        let src_linux = source_to_linux(&project.source)
            .ok_or_else(|| path_not_windows_err(&project.source))?;
        let state = Arc::clone(&self.state);
        let state_dir = self.state_dir.clone();
        let workspace = PathBuf::from(project.workspace.clone());
        let branch = project.branch.clone();
        let job_project = project;
        let work: Work = Box::new(move |cancel| {
            run_update(
                cancel,
                &state,
                &state_dir,
                &src_linux,
                &workspace,
                &branch,
                job_project,
            )
        });
        let job_id = self
            .runner
            .submit(JobKind::UpdateFromWindows, id, work)
            .map_err(|()| busy_err())?;
        Ok(JobRef { job_id })
    }

    pub fn remove(
        &self,
        id: ProjectId,
        delete_workspace: bool,
        force: bool,
    ) -> Result<JobRef, OpError> {
        let project = self.get_project(id)?;
        let state = Arc::clone(&self.state);
        let state_dir = self.state_dir.clone();
        let workspace = PathBuf::from(project.workspace.clone());
        let job_project = project;
        let work: Work = Box::new(move |cancel| {
            run_remove(
                cancel,
                &state,
                &state_dir,
                &workspace,
                delete_workspace,
                force,
                job_project,
            )
        });
        let job_id = self
            .runner
            .submit(JobKind::Remove, id, work)
            .map_err(|()| busy_err())?;
        Ok(JobRef { job_id })
    }

    pub fn relocate(&self, params: RelocateParams) -> Result<JobRef, OpError> {
        let RelocateParams { id, windows_path } = params;
        let project = self.get_project(id)?;
        let new_src_linux = source_to_linux(&windows_path)
            .ok_or_else(|| path_not_windows_err(&windows_path))?;
        if !git::is_repo(Path::new(&new_src_linux)) {
            return Err(not_a_git_repository_err(&windows_path));
        }
        let state = Arc::clone(&self.state);
        let state_dir = self.state_dir.clone();
        let workspace = PathBuf::from(project.workspace.clone());
        let job_project = project;
        let work: Work = Box::new(move |cancel| {
            run_relocate(
                cancel,
                &state,
                &state_dir,
                &workspace,
                &new_src_linux,
                &windows_path,
                job_project,
            )
        });
        let job_id = self
            .runner
            .submit(JobKind::Relocate, id, work)
            .map_err(|()| busy_err())?;
        Ok(JobRef { job_id })
    }

    pub fn rename(&self, params: RenameParams) -> Result<Project, OpError> {
        let RenameParams { id, name } = params;
        let mut project = self.get_project(id)?;
        project.name = name;
        store::save(&self.state_dir, &project).map_err(|e| {
            OpError::new(
                "git_failed",
                e.to_string(),
                "check the daemon's state directory permissions and \
                 try again",
            )
        })?;
        let _ = lock(&self.state).upsert_project(project.clone());
        Ok(project)
    }

    /// Recomputes `source_present` for every project from the
    /// filesystem. Called before a snapshot so the flag is always
    /// fresh, never trusted from disk or from a job that ran earlier.
    pub fn refresh_source_present(&self) {
        let mut state = lock(&self.state);
        for project in state.projects.values_mut() {
            project.source_present = source_to_linux(&project.source)
                .is_some_and(|linux| git::is_repo(Path::new(&linux)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use willie_proto::job::JobState;

    fn clock() -> String {
        "t".into()
    }

    fn ops(root: &Path) -> (Ops, Arc<Mutex<State>>) {
        let state = Arc::new(Mutex::new(State::default()));
        let (out, _h) = crate::outbound::Outbound::spawn(std::io::sink());
        let runner = Runner::new(Arc::clone(&state), out, clock);
        let ops = Ops::new(
            Arc::clone(&state),
            runner,
            root.join("state"),
            root.join("workspaces"),
            clock,
        );
        (ops, state)
    }

    fn init_repo(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        git::run(dir, &["init", "-b", "main"]).unwrap();
        git::run(dir, &["config", "user.email", "t@t"]).unwrap();
        git::run(dir, &["config", "user.name", "t"]).unwrap();
        fs::write(dir.join("f.txt"), "hi").unwrap();
        git::run(dir, &["add", "."]).unwrap();
        git::run(dir, &["commit", "-m", "init"]).unwrap();
    }

    fn wait_job_done(state: &Arc<Mutex<State>>) -> JobState {
        for _ in 0..300 {
            if let Some(j) = state.lock().unwrap().jobs.values().next()
                && !matches!(j.state, JobState::Running)
            {
                return j.state.clone();
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("job never finished");
    }

    /// A fresh clone (e.g. a workspace) carries no `user.*` identity of
    /// its own; a test that commits into one must set it first.
    fn configure_identity(dir: &Path) {
        git::run(dir, &["config", "user.email", "t@t"]).unwrap();
        git::run(dir, &["config", "user.name", "t"]).unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir()
            .join(format!("willie-ops-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn add_clones_and_marks_ready_with_the_windows_remote() {
        let root = scratch("add-ok");
        let src = root.join("src");
        init_repo(&src);
        let (ops, state) = ops(&root);
        let res = ops
            .add(AddParams {
                windows_path: src.to_string_lossy().into_owned(),
                name: None,
            })
            .unwrap();
        let done = wait_job_done(&state);
        assert!(matches!(done, JobState::Done), "{done:?}");
        let p = state.lock().unwrap().projects[&res.project_id].clone();
        assert!(matches!(p.state, ProjectState::Ready));
        assert!(git::is_repo(Path::new(&p.workspace)));
        let remotes = git::run(Path::new(&p.workspace), &["remote"]).unwrap();
        assert!(remotes.contains("windows"), "{remotes}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn add_refuses_a_non_repository() {
        let root = scratch("add-nonrepo");
        let plain = root.join("plain");
        fs::create_dir_all(&plain).unwrap();
        let (ops, _state) = ops(&root);
        let err = ops
            .add(AddParams {
                windows_path: plain.to_string_lossy().into_owned(),
                name: None,
            })
            .unwrap_err();
        assert_eq!(err.code, "not_a_git_repository");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn add_refuses_the_same_source_twice() {
        let root = scratch("add-dup");
        let src = root.join("src");
        init_repo(&src);
        let (ops, state) = ops(&root);
        let p = src.to_string_lossy().into_owned();
        ops.add(AddParams {
            windows_path: p.clone(),
            name: None,
        })
        .unwrap();
        wait_job_done(&state);
        let err = ops
            .add(AddParams {
                windows_path: p,
                name: None,
            })
            .unwrap_err();
        assert_eq!(err.code, "project_exists");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn add_refuses_a_relative_path() {
        let root = scratch("add-relative");
        let (ops, _state) = ops(&root);
        let err = ops
            .add(AddParams {
                windows_path: "relative/path".into(),
                name: None,
            })
            .unwrap_err();
        assert_eq!(err.code, "path_not_windows");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sync_refuses_a_dirty_windows_tree_without_touching_it() {
        let root = scratch("sync-dirty");
        let src = root.join("src");
        init_repo(&src);
        let (ops, state) = ops(&root);
        let res = ops
            .add(AddParams {
                windows_path: src.to_string_lossy().into_owned(),
                name: None,
            })
            .unwrap();
        wait_job_done(&state);
        // A commit in the workspace, and a dirty Windows tree.
        let ws = state.lock().unwrap().projects[&res.project_id]
            .workspace
            .clone();
        configure_identity(Path::new(&ws));
        fs::write(Path::new(&ws).join("f.txt"), "from agent").unwrap();
        git::run(Path::new(&ws), &["commit", "-am", "agent"]).unwrap();
        fs::write(src.join("f.txt"), "dirty").unwrap();
        // Clear the finished add job so wait sees the sync job.
        state.lock().unwrap().jobs.clear();
        ops.sync_to_windows(res.project_id).unwrap();
        let done = wait_job_done(&state);
        match done {
            JobState::Failed { code, .. } => {
                assert_eq!(code, "windows_tree_dirty")
            }
            other => panic!("{other:?}"),
        }
        // The Windows tree still holds the dirty content, untouched.
        assert_eq!(fs::read_to_string(src.join("f.txt")).unwrap(), "dirty");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sync_pushes_a_clean_workspace_commit_to_windows() {
        let root = scratch("sync-ok");
        let src = root.join("src");
        init_repo(&src);
        let (ops, state) = ops(&root);
        let res = ops
            .add(AddParams {
                windows_path: src.to_string_lossy().into_owned(),
                name: None,
            })
            .unwrap();
        wait_job_done(&state);
        let ws = state.lock().unwrap().projects[&res.project_id]
            .workspace
            .clone();
        configure_identity(Path::new(&ws));
        fs::write(Path::new(&ws).join("f.txt"), "from agent").unwrap();
        git::run(Path::new(&ws), &["commit", "-am", "agent"]).unwrap();
        state.lock().unwrap().jobs.clear();
        ops.sync_to_windows(res.project_id).unwrap();
        let done = wait_job_done(&state);
        assert!(matches!(done, JobState::Done), "{done:?}");
        assert_eq!(
            fs::read_to_string(src.join("f.txt")).unwrap(),
            "from agent"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn update_fast_forwards_the_workspace_from_windows() {
        let root = scratch("update-ok");
        let src = root.join("src");
        init_repo(&src);
        let (ops, state) = ops(&root);
        let res = ops
            .add(AddParams {
                windows_path: src.to_string_lossy().into_owned(),
                name: None,
            })
            .unwrap();
        wait_job_done(&state);
        fs::write(src.join("f.txt"), "from windows").unwrap();
        git::run(&src, &["commit", "-am", "windows"]).unwrap();
        state.lock().unwrap().jobs.clear();
        ops.update_from_windows(res.project_id).unwrap();
        let done = wait_job_done(&state);
        assert!(matches!(done, JobState::Done), "{done:?}");
        let ws = state.lock().unwrap().projects[&res.project_id]
            .workspace
            .clone();
        assert_eq!(
            fs::read_to_string(Path::new(&ws).join("f.txt")).unwrap(),
            "from windows"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn update_refuses_a_diverged_workspace() {
        let root = scratch("update-diverged");
        let src = root.join("src");
        init_repo(&src);
        let (ops, state) = ops(&root);
        let res = ops
            .add(AddParams {
                windows_path: src.to_string_lossy().into_owned(),
                name: None,
            })
            .unwrap();
        wait_job_done(&state);
        let ws = state.lock().unwrap().projects[&res.project_id]
            .workspace
            .clone();
        // Diverging commits on both sides.
        configure_identity(Path::new(&ws));
        fs::write(Path::new(&ws).join("f.txt"), "agent").unwrap();
        git::run(Path::new(&ws), &["commit", "-am", "agent"]).unwrap();
        fs::write(src.join("f.txt"), "windows").unwrap();
        git::run(&src, &["commit", "-am", "windows"]).unwrap();
        state.lock().unwrap().jobs.clear();
        ops.update_from_windows(res.project_id).unwrap();
        let done = wait_job_done(&state);
        match done {
            JobState::Failed { code, .. } => {
                assert_eq!(code, "workspace_diverged")
            }
            other => panic!("{other:?}"),
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn remove_refuses_a_dirty_workspace_without_force() {
        let root = scratch("remove-dirty");
        let src = root.join("src");
        init_repo(&src);
        let (ops, state) = ops(&root);
        let res = ops
            .add(AddParams {
                windows_path: src.to_string_lossy().into_owned(),
                name: None,
            })
            .unwrap();
        wait_job_done(&state);
        let ws = state.lock().unwrap().projects[&res.project_id]
            .workspace
            .clone();
        fs::write(Path::new(&ws).join("f.txt"), "dirty").unwrap();
        state.lock().unwrap().jobs.clear();
        ops.remove(res.project_id, true, false).unwrap();
        let done = wait_job_done(&state);
        match done {
            JobState::Failed { code, .. } => {
                assert_eq!(code, "workspace_dirty")
            }
            other => panic!("{other:?}"),
        }
        // Refused: the project is still registered and the workspace
        // still exists.
        assert!(state.lock().unwrap().projects.contains_key(&res.project_id));
        assert!(Path::new(&ws).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn remove_deletes_the_workspace_and_forgets_the_project() {
        let root = scratch("remove-ok");
        let src = root.join("src");
        init_repo(&src);
        let (ops, state) = ops(&root);
        let res = ops
            .add(AddParams {
                windows_path: src.to_string_lossy().into_owned(),
                name: None,
            })
            .unwrap();
        wait_job_done(&state);
        let ws = state.lock().unwrap().projects[&res.project_id]
            .workspace
            .clone();
        state.lock().unwrap().jobs.clear();
        ops.remove(res.project_id, true, false).unwrap();
        let done = wait_job_done(&state);
        assert!(matches!(done, JobState::Done), "{done:?}");
        assert!(!state.lock().unwrap().projects.contains_key(&res.project_id));
        assert!(!Path::new(&ws).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn relocate_refuses_an_unrelated_history() {
        let root = scratch("relocate-unrelated");
        let src = root.join("src");
        init_repo(&src);
        let (ops, state) = ops(&root);
        let res = ops
            .add(AddParams {
                windows_path: src.to_string_lossy().into_owned(),
                name: None,
            })
            .unwrap();
        wait_job_done(&state);
        // A genuinely unrelated repository: different tree, different
        // message, so its root commit cannot collide by content with
        // `src`'s even if both are created within the same second.
        let other = root.join("other");
        fs::create_dir_all(&other).unwrap();
        git::run(&other, &["init", "-b", "main"]).unwrap();
        configure_identity(&other);
        fs::write(other.join("other.txt"), "unrelated content").unwrap();
        git::run(&other, &["add", "."]).unwrap();
        git::run(&other, &["commit", "-m", "unrelated init"]).unwrap();
        state.lock().unwrap().jobs.clear();
        ops.relocate(RelocateParams {
            id: res.project_id,
            windows_path: other.to_string_lossy().into_owned(),
        })
        .unwrap();
        let done = wait_job_done(&state);
        match done {
            JobState::Failed { code, .. } => {
                assert_eq!(code, "source_unrelated")
            }
            other => panic!("{other:?}"),
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn relocate_points_the_project_at_the_new_source() {
        let root = scratch("relocate-ok");
        let src = root.join("src");
        init_repo(&src);
        let (ops, state) = ops(&root);
        let res = ops
            .add(AddParams {
                windows_path: src.to_string_lossy().into_owned(),
                name: None,
            })
            .unwrap();
        wait_job_done(&state);
        let moved = root.join("moved");
        git::clone(&src, &moved).unwrap();
        state.lock().unwrap().jobs.clear();
        ops.relocate(RelocateParams {
            id: res.project_id,
            windows_path: moved.to_string_lossy().into_owned(),
        })
        .unwrap();
        let done = wait_job_done(&state);
        assert!(matches!(done, JobState::Done), "{done:?}");
        let p = state.lock().unwrap().projects[&res.project_id].clone();
        assert_eq!(p.source, moved.to_string_lossy());
        let remotes =
            git::run(Path::new(&p.workspace), &["remote", "-v"]).unwrap();
        assert!(remotes.contains(&moved.to_string_lossy().into_owned()));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_changes_the_name_and_is_synchronous() {
        let root = scratch("rename-ok");
        let src = root.join("src");
        init_repo(&src);
        let (ops, state) = ops(&root);
        let res = ops
            .add(AddParams {
                windows_path: src.to_string_lossy().into_owned(),
                name: None,
            })
            .unwrap();
        wait_job_done(&state);
        let renamed = ops
            .rename(RenameParams {
                id: res.project_id,
                name: "New Name".into(),
            })
            .unwrap();
        assert_eq!(renamed.name, "New Name");
        assert_eq!(
            state.lock().unwrap().projects[&res.project_id].name,
            "New Name"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_refuses_an_unknown_project() {
        let root = scratch("rename-missing");
        let (ops, _state) = ops(&root);
        let err = ops
            .rename(RenameParams {
                id: ProjectId::new(),
                name: "x".into(),
            })
            .unwrap_err();
        assert_eq!(err.code, "project_not_found");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn refresh_source_present_reflects_a_removed_source() {
        let root = scratch("refresh");
        let src = root.join("src");
        init_repo(&src);
        let (ops, state) = ops(&root);
        let res = ops
            .add(AddParams {
                windows_path: src.to_string_lossy().into_owned(),
                name: None,
            })
            .unwrap();
        wait_job_done(&state);
        ops.refresh_source_present();
        assert!(state.lock().unwrap().projects[&res.project_id].source_present);
        fs::remove_dir_all(&src).unwrap();
        ops.refresh_source_present();
        assert!(
            !state.lock().unwrap().projects[&res.project_id].source_present
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_busy_project_refuses_a_second_job() {
        let root = scratch("busy");
        let src = root.join("src");
        init_repo(&src);
        let (ops, state) = ops(&root);
        let res = ops
            .add(AddParams {
                windows_path: src.to_string_lossy().into_owned(),
                name: None,
            })
            .unwrap();
        wait_job_done(&state);
        state.lock().unwrap().jobs.clear();
        // Saturate the project's single job slot directly through the
        // runner `ops` shares, then try a normal op on top of it.
        let (tx, rx) = mpsc::channel::<()>();
        let held = ops.runner.submit(
            JobKind::SyncToWindows,
            res.project_id,
            Box::new(move |_| {
                let _ = rx.recv();
                Ok(String::new())
            }),
        );
        assert!(held.is_ok());
        let err = ops.sync_to_windows(res.project_id).unwrap_err();
        assert_eq!(err.code, "project_busy");
        drop(tx);
        let _ = fs::remove_dir_all(&root);
    }
}
