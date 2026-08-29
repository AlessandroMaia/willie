import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useRef, useState } from "react";
import type { Problem } from "../lib/engine";
import {
  isProblem,
  onDaemonEvent,
  projects as projectsApi,
  sessions as sessionsApi,
} from "../lib/engine";
import { latestJobFor } from "../lib/jobs";
import type { Candidate, Job, JobKind, Project, Snapshot } from "../lib/proto";
import { liveCount } from "../lib/sessions";
import { applyEvent, needsResnapshot } from "../lib/state";

function wslPathFor(slug: string): string {
  return `\\\\wsl.localhost\\willie\\home\\willie\\projects\\${slug}`;
}

const RETRYABLE_KINDS: JobKind[] = ["sync_to_windows", "update_from_windows"];

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

function asProblem(error: unknown): Problem {
  return isProblem(error)
    ? error
    : { code: "unknown", message: String(error), remediation: "" };
}

interface StateChipProps {
  project: Project;
  job: Job | undefined;
  onRetry: (job: Job) => void;
}

function StateChip({ project, job, onRetry }: StateChipProps) {
  if (project.state.state === "failed") {
    return (
      <div className="chip chip-failed">
        <code>{project.state.code}</code>
        <span>{project.state.message}</span>
        {project.state.remediation && (
          <div className="muted">→ {project.state.remediation}</div>
        )}
      </div>
    );
  }
  if (job && job.state.state === "failed") {
    /* `workspace_dirty` only ever arrives this way: `project_remove`
     * resolves the instant the job is queued, so the dirty-workspace
     * refusal is never a promise rejection the confirm dialog can
     * catch — it is a `job_changed` event, exactly like any other job
     * outcome. The one-click "Remove anyway" re-submits with `force`,
     * the same safe-resubmit shape as a plain retry. */
    const forceRemove =
      job.kind === "remove" && job.state.code === "workspace_dirty";
    const canRetry = forceRemove || RETRYABLE_KINDS.includes(job.kind);
    return (
      <div className="chip chip-failed">
        <code>{job.state.code}</code>
        <span>{job.state.message}</span>
        {job.state.remediation && (
          <div className="muted">→ {job.state.remediation}</div>
        )}
        {canRetry && (
          <button type="button" onClick={() => onRetry(job)}>
            {forceRemove ? "Remove anyway" : "Retry"}
          </button>
        )}
      </div>
    );
  }
  if (project.state.state === "preparing" || job?.state.state === "running") {
    return (
      <div className="chip chip-busy">
        <span className="spinner" aria-hidden="true" />
        <span>{job?.log_tail || "working…"}</span>
      </div>
    );
  }
  return <span className="chip chip-ready">ready</span>;
}

export function Projects() {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const snapRef = useRef<Snapshot | null>(null);
  const [problem, setProblem] = useState<Problem | null>(null);
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
   * `daemon://event` — see `StateChip`) resubmits with the same
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

  const loadSnapshot = useCallback(() => {
    projectsApi
      .snapshot()
      .then((next) => {
        snapRef.current = next;
        setSnap(next);
        /* A resnapshot can be the only place a removal is ever
         * observed (a missed `project_removed` event forces a full
         * refetch instead of an incremental apply), so prune
         * `removeAttempts` here too, against the fresh project list. */
        const liveIds = new Set(next.projects.map((p) => p.id));
        setRemoveAttempts((prev) => {
          const stale = [...prev.keys()].filter((id) => !liveIds.has(id));
          if (stale.length === 0) return prev;
          const pruned = new Map(prev);
          for (const id of stale) pruned.delete(id);
          return pruned;
        });
      })
      .catch((error: unknown) => setProblem(asProblem(error)));
  }, []);

  const loadRoots = useCallback(() => {
    projectsApi
      .roots()
      .then(setRoots)
      .catch((error: unknown) => setProblem(asProblem(error)));
  }, []);

  useEffect(() => {
    loadSnapshot();
    loadRoots();
  }, [loadSnapshot, loadRoots]);

  /* `applyEvent`/`needsResnapshot` are pure, so the decision to fold an
   * event in place versus re-fetching lives here, next to the only
   * mutable copy of the snapshot; `snapRef` lets the handler read the
   * latest value synchronously without re-subscribing on every apply. */
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    onDaemonEvent((ev) => {
      const current = snapRef.current;
      if (current === null || needsResnapshot(current, ev)) {
        loadSnapshot();
        return;
      }
      const next = applyEvent(current, ev);
      snapRef.current = next;
      setSnap(next);
      /* `removeAttempts` remembers a choice per project id only for as
       * long as `forceRemoveJob` might need it; once the project is
       * gone there is nothing left to force-remove, so drop the entry
       * here rather than let the map grow for the rest of the session. */
      if (ev.kind === "project_removed") {
        setRemoveAttempts((prev) => {
          if (!prev.has(ev.id)) return prev;
          const next = new Map(prev);
          next.delete(ev.id);
          return next;
        });
      }
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [loadSnapshot]);

  async function run(
    id: string,
    action: () => Promise<unknown>,
  ): Promise<boolean> {
    setBusyId(id);
    setProblem(null);
    try {
      await action();
      return true;
    } catch (error) {
      setProblem(asProblem(error));
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
    setProblem(null);
    try {
      const found = await projectsApi.discover();
      setCandidates(found);
      setSelected(new Set());
    } catch (error) {
      setProblem(asProblem(error));
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
    setProblem(null);
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
      setProblem({
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
    runRow(project.id, project.id, async () => {
      const result = await sessionsApi.open(project.id);
      setOpenNotice(project.id, result.terminal_problem ?? null);
    });
  }

  function syncToWindows(project: Project) {
    runRow(project.id, project.id, () => projectsApi.syncToWindows(project.id));
  }

  function updateFromWindows(project: Project) {
    runRow(project.id, project.id, () =>
      projectsApi.updateFromWindows(project.id),
    );
  }

  function cancelJobFor(job: Job) {
    runRow(projectJobId(job), job.id, () => projectsApi.cancelJob(job.id));
  }

  /* A job outcome the daemon reports asynchronously (`workspace_dirty`
   * on `remove`) resubmits with `force`, using the `deleteWorkspace`
   * choice remembered from the submission that failed. Everything else
   * retryable just re-runs the same call with the project id. */
  function forceRemoveJob(job: Job) {
    const projectId = projectJobId(job);
    const keepWorkspace = removeAttempts.get(projectId) ?? true;
    runRow(projectId, projectId, () =>
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
      runRow(projectId, projectId, fn);
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
   * is surfaced on the project's row (see `StateChip` and
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
   * failure is picked up by `latestJobFor`/`StateChip` on the row like
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

      {problem && (
        <section className="problem" role="alert">
          <strong>{problem.code}</strong> — {problem.message}
          {problem.remediation && (
            <div className="muted">→ {problem.remediation}</div>
          )}
        </section>
      )}

      <section className="roots">
        <h2>Roots</h2>
        <ul>
          {roots.map((root) => (
            <li key={root}>
              <span>{root}</span>
              <button type="button" onClick={() => removeRoot(root)}>
                Remove
              </button>
            </li>
          ))}
        </ul>
        <div className="actions">
          <input
            value={newRoot}
            onChange={(e) => setNewRoot(e.target.value)}
            placeholder="C:\github\..."
            aria-label="new root"
          />
          <button
            type="button"
            disabled={newRoot.trim() === "" || busyId === "roots"}
            onClick={addRoot}
          >
            Add root
          </button>
          <button type="button" disabled={discovering} onClick={runDiscover}>
            {discovering ? "Discovering…" : "Discover"}
          </button>
        </div>
      </section>

      {candidates !== null && (
        <section className="discover">
          <h2>Discovered</h2>
          {candidates.length === 0 ? (
            <p className="muted">No repositories found under the roots.</p>
          ) : (
            <>
              <ul>
                {candidates.map((c) => (
                  <li key={c.path} className="candidate">
                    <label>
                      <input
                        type="checkbox"
                        checked={selected.has(c.path)}
                        onChange={() => toggleCandidate(c.path)}
                      />
                      {c.name}
                      <span className="muted"> — {c.path}</span>
                    </label>
                  </li>
                ))}
              </ul>
              <button
                type="button"
                disabled={selected.size === 0 || busyId === "add-selected"}
                onClick={addSelected}
              >
                Add selected
              </button>
            </>
          )}
        </section>
      )}

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
            return (
              <div key={project.id} className="project-row">
                <div className="project-row-main">
                  {isEditing ? (
                    <span className="rename">
                      <input
                        value={editingName}
                        onChange={(e) => setEditingName(e.target.value)}
                        aria-label="project name"
                      />
                      <button type="button" onClick={() => saveRename(project)}>
                        Save
                      </button>
                      <button type="button" onClick={cancelRename}>
                        Cancel
                      </button>
                    </span>
                  ) : (
                    <span className="project-name">
                      <strong>{project.name}</strong>
                      <button
                        type="button"
                        onClick={() => startRename(project)}
                      >
                        Rename
                      </button>
                    </span>
                  )}
                  <StateChip project={project} job={job} onRetry={retry} />
                  {live > 0 && (
                    <span
                      className="badge badge-live"
                      title={`${live} live session${live === 1 ? "" : "s"}`}
                    >
                      {live} live
                    </span>
                  )}
                </div>

                <div className="project-row-detail muted">
                  <div>source: {project.source}</div>
                  <div>
                    workspace: <code>{path}</code>
                    <button type="button" onClick={() => copyPath(path)}>
                      Copy
                    </button>
                    <button
                      type="button"
                      onClick={() => openInExplorer(project, path)}
                    >
                      Open in Explorer
                    </button>
                  </div>
                  <div>branch: {project.branch}</div>
                  {!project.source_present && (
                    <div className="badge badge-warning">
                      source missing
                      <button
                        type="button"
                        onClick={() => openRelocateDialog(project)}
                      >
                        Relocate
                      </button>
                    </div>
                  )}
                </div>

                {rowProblem && (
                  <div className="problem" role="alert">
                    <strong>{rowProblem.code}</strong> — {rowProblem.message}
                    {rowProblem.remediation && (
                      <div className="muted">→ {rowProblem.remediation}</div>
                    )}
                  </div>
                )}

                {openNotice && (
                  <div className="notice" role="status">
                    <span>{openNotice.message}</span>
                    {openNotice.remediation && (
                      <div className="muted">→ {openNotice.remediation}</div>
                    )}
                  </div>
                )}

                <div className="actions">
                  <button
                    type="button"
                    disabled={
                      isBusy || project.state.state !== "ready" || jobRunning
                    }
                    onClick={() => openSession(project)}
                  >
                    Open session
                  </button>
                  <button
                    type="button"
                    disabled={
                      isBusy || project.state.state !== "ready" || jobRunning
                    }
                    onClick={() => syncToWindows(project)}
                  >
                    Send to Windows
                  </button>
                  <button
                    type="button"
                    disabled={
                      isBusy || project.state.state !== "ready" || jobRunning
                    }
                    onClick={() => updateFromWindows(project)}
                  >
                    Update from Windows
                  </button>
                  {job && jobRunning && (
                    <button type="button" onClick={() => cancelJobFor(job)}>
                      Cancel
                    </button>
                  )}
                  <button
                    type="button"
                    onClick={() => openRemoveDialog(project)}
                  >
                    Remove
                  </button>
                  {isBusy && <span className="muted">working…</span>}
                </div>
              </div>
            );
          })
        )}
      </section>

      {removing && (
        <div className="modal-backdrop">
          <div className="modal" role="dialog" aria-modal="true">
            <h2>Remove “{removing.name}”?</h2>
            <label>
              <input
                type="checkbox"
                checked={deleteWorkspace}
                onChange={(e) => setDeleteWorkspace(e.target.checked)}
              />
              Delete the workspace clone too
            </label>
            {removeProblem && (
              <div className="problem" role="alert">
                <strong>{removeProblem.code}</strong> — {removeProblem.message}
                {removeProblem.remediation && (
                  <div className="muted">→ {removeProblem.remediation}</div>
                )}
              </div>
            )}
            <p className="muted">
              A workspace with uncommitted changes is refused; if that happens
              the project's row will offer a one-click "Remove anyway" once the
              daemon reports it.
            </p>
            <div className="actions">
              <button type="button" onClick={confirmRemove}>
                Remove
              </button>
              <button type="button" onClick={closeRemoveDialog}>
                Cancel
              </button>
            </div>
          </div>
        </div>
      )}

      {relocating && (
        <div className="modal-backdrop">
          <div className="modal" role="dialog" aria-modal="true">
            <h2>Relocate “{relocating.name}”</h2>
            <div className="actions">
              <input
                value={relocatePath}
                onChange={(e) => setRelocatePath(e.target.value)}
                placeholder="C:\github\..."
                aria-label="new source path"
              />
              <button type="button" onClick={pickRelocateFolder}>
                Browse…
              </button>
            </div>
            {relocateProblem && (
              <div className="problem" role="alert">
                <strong>{relocateProblem.code}</strong> —{" "}
                {relocateProblem.message}
                {relocateProblem.remediation && (
                  <div className="muted">→ {relocateProblem.remediation}</div>
                )}
              </div>
            )}
            <div className="actions">
              <button
                type="button"
                disabled={relocatePath.trim() === ""}
                onClick={confirmRelocate}
              >
                Relocate
              </button>
              <button type="button" onClick={closeRelocateDialog}>
                Cancel
              </button>
            </div>
          </div>
        </div>
      )}
    </main>
  );
}
