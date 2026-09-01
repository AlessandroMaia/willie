import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useState } from "react";
import { ProblemAlert } from "@/components/problem-alert";
import { DiscoverPanel } from "@/features/projects/discover-panel";
import { ProjectRow } from "@/features/projects/project-row";
import { RelocateProjectDialog } from "@/features/projects/relocate-project-dialog";
import { RemoveProjectDialog } from "@/features/projects/remove-project-dialog";
import { RootsPanel } from "@/features/projects/roots-panel";
import { latestJobFor } from "@/lib/domain/jobs";
import { isLive, liveCount } from "@/lib/domain/sessions";
import type { Problem } from "@/lib/ipc";
import { projects as projectsApi, sessions as sessionsApi } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import type { Candidate, Job, Project } from "@/lib/proto";
import { useSnapshot } from "@/store/use-snapshot";

function wslPathFor(slug: string): string {
  return `\\\\wsl.localhost\\willie\\home\\willie\\projects\\${slug}`;
}

/* A job carries no memory of the parameters that started it (a failed
 * `relocate` does not remember the path it tried), so only the kinds
 * that can be safely re-run with just the project id offer a Retry. */
function retryFn(job: Job): (() => Promise<unknown>) | null {
  switch (job.kind) {
    case "sync_to_windows":
      return () => projectsApi.syncToWindows(projectJobId(job));
    case "update_from_windows":
      return () => projectsApi.updateFromWindows(projectJobId(job));
    default:
      return null;
  }
}

/* Every job reaching the functions below came from `latestJobFor`,
 * matched to one project's id, so it always carries that id back; a
 * tool job (no project) never appears in a project row's controls. */
function projectJobId(job: Job): string {
  const id = job.project_id;
  if (id === undefined) throw new Error(`job ${job.id} has no project_id`);
  return id;
}

export function ProjectsScreen() {
  const store = useSnapshot();
  const snap = store.snapshot;
  const [local, setLocal] = useState<Problem | null>(null);
  const problem = local ?? store.problem;
  const [roots, setRoots] = useState<string[]>([]);
  const [newRoot, setNewRoot] = useState("");
  const [addPath, setAddPath] = useState("");
  const [addName, setAddName] = useState("");
  const [discovering, setDiscovering] = useState(false);
  const [candidates, setCandidates] = useState<Candidate[] | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busyId, setBusyId] = useState<string | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingName, setEditingName] = useState("");
  const [removing, setRemoving] = useState<Project | null>(null);
  const [deleteWorkspace, setDeleteWorkspace] = useState(true);
  const [removeProblem, setRemoveProblem] = useState<Problem | null>(null);
  /* The `deleteWorkspace` choice a remove submission used, remembered
   * per project so a later one-click "force" retry (from a
   * `workspace_dirty` job failure, surfaced only through
   * `daemon://event` — see `ProjectStateChip`) resubmits with the same
   * choice instead of asking the user again. */
  const [removeAttempts, setRemoveAttempts] = useState<Map<string, boolean>>(
    new Map(),
  );
  const [relocating, setRelocating] = useState<Project | null>(null);
  const [relocatePath, setRelocatePath] = useState("");
  const [relocateProblem, setRelocateProblem] = useState<Problem | null>(null);
  /* Synchronous RPC-level rejects that belong to one project (a job
   * already running, a cancel or retry that failed) — shown on that
   * project's row, never in the page-level banner below, which is
   * reserved for genuinely global actions (roots, discover, add). */
  const [rowProblems, setRowProblems] = useState<Map<string, Problem>>(
    new Map(),
  );
  /* A session that opened successfully but whose terminal tab could not
   * be launched (`SessionOpened.terminal_problem`) — the session is
   * alive, so this is kept apart from `rowProblems` and rendered as a
   * dismissable notice instead of a row failure. */
  const [openNotices, setOpenNotices] = useState<Map<string, Problem>>(
    new Map(),
  );

  /* A resnapshot can be the only place a removal is ever observed (a
   * missed `project_removed` event forces the store to refetch instead
   * of an incremental apply), so prune `removeAttempts` whenever the
   * snapshot changes, against the fresh project list — not only when
   * this component happens to see the removal event go by. */
  useEffect(() => {
    if (!snap) return;
    const liveIds = new Set(snap.projects.map((p) => p.id));
    setRemoveAttempts((prev) => {
      const stale = [...prev.keys()].filter((id) => !liveIds.has(id));
      if (stale.length === 0) return prev;
      const pruned = new Map(prev);
      for (const id of stale) pruned.delete(id);
      return pruned;
    });
  }, [snap]);

  const loadRoots = useCallback(() => {
    projectsApi
      .roots()
      .then(setRoots)
      .catch((error: unknown) => setLocal(asProblem(error)));
  }, []);

  useEffect(() => {
    loadRoots();
  }, [loadRoots]);

  async function run(
    id: string,
    action: () => Promise<unknown>,
  ): Promise<boolean> {
    setBusyId(id);
    setLocal(null);
    try {
      await action();
      return true;
    } catch (error) {
      setLocal(asProblem(error));
      return false;
    } finally {
      setBusyId(null);
    }
  }

  function setRowProblem(projectId: string, problem: Problem | null) {
    setRowProblems((prev) => {
      const next = new Map(prev);
      if (problem) next.set(projectId, problem);
      else next.delete(projectId);
      return next;
    });
  }

  function setOpenNotice(projectId: string, problem: Problem | null) {
    setOpenNotices((prev) => {
      const next = new Map(prev);
      if (problem) next.set(projectId, problem);
      else next.delete(projectId);
      return next;
    });
  }

  /* Same shape as `run`, but a synchronous reject lands on the project's
   * own row instead of the page banner — the failure belongs to one
   * project, so a toast with no project name is the wrong place for it. */
  async function runRow(
    projectId: string,
    busyKey: string,
    action: () => Promise<unknown>,
  ): Promise<boolean> {
    setBusyId(busyKey);
    setRowProblem(projectId, null);
    try {
      await action();
      return true;
    } catch (error) {
      setRowProblem(projectId, asProblem(error));
      return false;
    } finally {
      setBusyId(null);
    }
  }

  async function addRoot() {
    const value = newRoot.trim();
    if (value === "" || roots.includes(value)) return;
    const next = [...roots, value];
    const ok = await run("roots", () => projectsApi.setRoots(next));
    if (ok) {
      setRoots(next);
      setNewRoot("");
    }
  }

  async function removeRoot(root: string) {
    const next = roots.filter((r) => r !== root);
    const ok = await run("roots", () => projectsApi.setRoots(next));
    if (ok) setRoots(next);
  }

  async function runDiscover() {
    setDiscovering(true);
    setLocal(null);
    try {
      const found = await projectsApi.discover();
      setCandidates(found);
      setSelected(new Set());
    } catch (error) {
      setLocal(asProblem(error));
    } finally {
      setDiscovering(false);
    }
  }

  function toggleCandidate(path: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }

  async function addSelected() {
    const paths = Array.from(selected);
    if (paths.length === 0) return;
    setBusyId("add-selected");
    setLocal(null);
    const failures: string[] = [];
    for (const path of paths) {
      try {
        await projectsApi.add(path);
      } catch (error) {
        failures.push(`${path}: ${asProblem(error).message}`);
      }
    }
    setBusyId(null);
    if (failures.length > 0) {
      setLocal({
        code: "add_failed",
        message: failures.join("; "),
        remediation: "",
      });
    } else {
      setCandidates(null);
      setSelected(new Set());
    }
  }

  async function pickFolder() {
    const picked = await open({ directory: true });
    if (typeof picked === "string") setAddPath(picked);
  }

  async function addByPath() {
    const path = addPath.trim();
    if (path === "") return;
    const name = addName.trim();
    const ok = await run("add-path", () =>
      projectsApi.add(path, name === "" ? undefined : name),
    );
    if (ok) {
      setAddPath("");
      setAddName("");
    }
  }

  function startRename(project: Project) {
    setEditingId(project.id);
    setEditingName(project.name);
  }

  function cancelRename() {
    setEditingId(null);
    setEditingName("");
  }

  async function saveRename(project: Project) {
    const name = editingName.trim();
    if (name === "" || name === project.name) {
      setEditingId(null);
      return;
    }
    const ok = await runRow(project.id, project.id, () =>
      projectsApi.rename(project.id, name),
    );
    if (ok) setEditingId(null);
  }

  /* `runRow` already routes a thrown `Problem` (the create itself
   * failing — `project_busy`, `harness_not_installed`, …) into
   * `rowProblems`. A `terminal_problem` only ever rides along on a
   * *successful* open — the session is alive, just no tab launched — so
   * it is pulled out of the result here and kept in `openNotices`
   * instead of being treated as a row failure. */
  function openSession(project: Project) {
    setOpenNotice(project.id, null);
    void runRow(project.id, project.id, async () => {
      const result = await sessionsApi.open(project.id);
      setOpenNotice(project.id, result.terminal_problem ?? null);
    });
  }

  /* Same shape as `openSession`: a `terminal_problem` on an otherwise
   * successful resume is a live-session notice, not a row failure; a
   * thrown `Problem` (`session_already_live`, `harness_cannot_resume`,
   * …) falls through to `runRow`'s row-failure handling like any other
   * action. */
  function resumeSession(project: Project) {
    setOpenNotice(project.id, null);
    void runRow(project.id, project.id, async () => {
      const result = await sessionsApi.resume(project.id);
      setOpenNotice(project.id, result.terminal_problem ?? null);
    });
  }

  function syncToWindows(project: Project) {
    void runRow(project.id, project.id, () =>
      projectsApi.syncToWindows(project.id),
    );
  }

  function updateFromWindows(project: Project) {
    void runRow(project.id, project.id, () =>
      projectsApi.updateFromWindows(project.id),
    );
  }

  function cancelJobFor(job: Job) {
    void runRow(projectJobId(job), job.id, () => projectsApi.cancelJob(job.id));
  }

  /* A job outcome the daemon reports asynchronously (`workspace_dirty`
   * on `remove`) resubmits with `force`, using the `deleteWorkspace`
   * choice remembered from the submission that failed. Everything else
   * retryable just re-runs the same call with the project id. */
  function forceRemoveJob(job: Job) {
    const projectId = projectJobId(job);
    const keepWorkspace = removeAttempts.get(projectId) ?? true;
    void runRow(projectId, projectId, () =>
      projectsApi.remove(projectId, keepWorkspace, true),
    );
  }

  function retry(job: Job) {
    if (
      job.kind === "remove" &&
      job.state.state === "failed" &&
      job.state.code === "workspace_dirty"
    ) {
      forceRemoveJob(job);
      return;
    }
    const fn = retryFn(job);
    if (fn) {
      const projectId = projectJobId(job);
      void runRow(projectId, projectId, fn);
    }
  }

  function copyPath(path: string) {
    navigator.clipboard?.writeText(path).catch(() => {
      /* best effort — the path is also shown as plain text */
    });
  }

  function openInExplorer(project: Project, path: string) {
    projectsApi
      .openInExplorer(path)
      .catch((error: unknown) => setRowProblem(project.id, asProblem(error)));
  }

  function openRemoveDialog(project: Project) {
    setRemoving(project);
    setDeleteWorkspace(true);
    setRemoveProblem(null);
  }

  function closeRemoveDialog() {
    setRemoving(null);
    setRemoveProblem(null);
  }

  /* Submits the job and, on synchronous acceptance, closes the dialog
   * immediately — `project_remove` resolves as soon as the job is
   * queued, well before the daemon has actually looked at the
   * workspace. A `workspace_dirty` refusal is not a rejection this
   * `catch` will ever see: it lands later as a `job_changed` event and
   * is surfaced on the project's row (see `ProjectStateChip` and
   * `forceRemoveJob`), not here. Only a fast validation — the project
   * not existing, or a job already running for it — rejects
   * synchronously and keeps the dialog open to show it. */
  async function confirmRemove() {
    if (!removing) return;
    const project = removing;
    setRemoveProblem(null);
    try {
      await projectsApi.remove(project.id, deleteWorkspace, false);
      setRemoveAttempts((prev) =>
        new Map(prev).set(project.id, deleteWorkspace),
      );
      setRemoving(null);
    } catch (error) {
      setRemoveProblem(asProblem(error));
    }
  }

  function openRelocateDialog(project: Project) {
    setRelocating(project);
    setRelocatePath(project.source);
    setRelocateProblem(null);
  }

  function closeRelocateDialog() {
    setRelocating(null);
    setRelocateProblem(null);
  }

  async function pickRelocateFolder() {
    const picked = await open({ directory: true });
    if (typeof picked === "string") setRelocatePath(picked);
  }

  /* Same synchronous/asynchronous split as `confirmRemove`:
   * `path_not_windows` and `not_a_git_repository` are fast validations
   * `project_relocate` rejects with before a job ever starts, so they
   * belong here, in the still-open dialog. `source_unrelated` is only
   * decided once the job diffs histories, so it is never a rejection
   * this `catch` sees — closing the dialog on synchronous acceptance
   * does not mean the relocate succeeded. That later `job_changed`
   * failure is picked up by `latestJobFor`/`ProjectStateChip` on the row like
   * any other job outcome; `source_present` stays false (relocate only
   * updates it on success), so the row's "Relocate" badge is still
   * there for the user to try again — no separate one-click retry is
   * safe here since the job carries no memory of the path it tried. */
  async function confirmRelocate() {
    if (!relocating) return;
    const path = relocatePath.trim();
    if (path === "") return;
    setRelocateProblem(null);
    try {
      await projectsApi.relocate(relocating.id, path);
      setRelocating(null);
    } catch (error) {
      setRelocateProblem(asProblem(error));
    }
  }

  return (
    <main className="projects">
      <header>
        <h1>Projects</h1>
      </header>

      {problem && <ProblemAlert problem={problem} />}

      <RootsPanel
        roots={roots}
        newRoot={newRoot}
        onNewRootChange={setNewRoot}
        busy={busyId === "roots"}
        onAdd={addRoot}
        onRemove={removeRoot}
        discovering={discovering}
        onDiscover={runDiscover}
      />

      <DiscoverPanel
        candidates={candidates}
        selected={selected}
        busy={busyId === "add-selected"}
        onToggle={toggleCandidate}
        onAddSelected={addSelected}
      />

      <section className="add-project">
        <h2>Add by path</h2>
        <div className="actions">
          <input
            value={addPath}
            onChange={(e) => setAddPath(e.target.value)}
            placeholder="C:\github\..."
            aria-label="project path"
          />
          <button type="button" onClick={pickFolder}>
            Browse…
          </button>
          <input
            value={addName}
            onChange={(e) => setAddName(e.target.value)}
            placeholder="name (optional)"
            aria-label="project name"
          />
          <button
            type="button"
            disabled={addPath.trim() === "" || busyId === "add-path"}
            onClick={addByPath}
          >
            Add
          </button>
        </div>
      </section>

      <section className="project-list">
        <h2>Registered</h2>
        {snap === null ? (
          <p className="muted">Loading projects…</p>
        ) : snap.projects.length === 0 ? (
          <p className="muted">No projects yet.</p>
        ) : (
          snap.projects.map((project) => {
            const job = latestJobFor(snap.jobs, project.id);
            const isEditing = editingId === project.id;
            const isBusy = busyId === project.id || busyId === job?.id;
            const jobRunning = job?.state.state === "running";
            const path = wslPathFor(project.slug);
            const rowProblem = rowProblems.get(project.id) ?? null;
            const openNotice = openNotices.get(project.id) ?? null;
            const live = liveCount(snap.sessions, project.id);
            /* Resume needs a finished conversation to resume and no live
             * one already occupying the project — both read straight off
             * the snapshot, never a locally-tracked flag. */
            const hasFinishedSession = snap.sessions.some(
              (s) => s.project_id === project.id && !isLive(s),
            );
            const canResume = live === 0 && hasFinishedSession;
            return (
              <ProjectRow
                key={project.id}
                project={project}
                job={job}
                isEditing={isEditing}
                editingName={editingName}
                isBusy={isBusy}
                jobRunning={jobRunning}
                path={path}
                rowProblem={rowProblem}
                openNotice={openNotice}
                live={live}
                canResume={canResume}
                onEditingNameChange={setEditingName}
                onStartRename={() => startRename(project)}
                onSaveRename={() => saveRename(project)}
                onCancelRename={cancelRename}
                onRetry={retry}
                onCopyPath={() => copyPath(path)}
                onOpenInExplorer={() => openInExplorer(project, path)}
                onOpenRelocateDialog={() => openRelocateDialog(project)}
                onOpenSession={() => openSession(project)}
                onResumeSession={() => resumeSession(project)}
                onSyncToWindows={() => syncToWindows(project)}
                onUpdateFromWindows={() => updateFromWindows(project)}
                onCancelJob={cancelJobFor}
                onOpenRemoveDialog={() => openRemoveDialog(project)}
              />
            );
          })
        )}
      </section>

      <RemoveProjectDialog
        project={removing}
        deleteWorkspace={deleteWorkspace}
        problem={removeProblem}
        onToggleWorkspace={setDeleteWorkspace}
        onConfirm={confirmRemove}
        onCancel={closeRemoveDialog}
      />

      <RelocateProjectDialog
        project={relocating}
        path={relocatePath}
        problem={relocateProblem}
        onPathChange={setRelocatePath}
        onBrowse={pickRelocateFolder}
        onConfirm={confirmRelocate}
        onCancel={closeRelocateDialog}
      />
    </main>
  );
}
