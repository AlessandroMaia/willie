import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useRef, useState } from "react";
import type { Problem } from "../lib/engine";
import {
  isProblem,
  onDaemonEvent,
  projects as projectsApi,
} from "../lib/engine";
import type { Candidate, Job, JobKind, Project, Snapshot } from "../lib/proto";
import { applyEvent, needsResnapshot } from "../lib/state";

/* The UI never computes project or job truth: every value below is
 * derived, read-only, from the snapshot the store hands us. */
function latestJobFor(jobs: Job[], projectId: string): Job | undefined {
  let latest: Job | undefined;
  for (const job of jobs) {
    if (job.project_id !== projectId) continue;
    if (!latest || job.started_at > latest.started_at) latest = job;
  }
  return latest;
}

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
      return () => projectsApi.syncToWindows(job.project_id);
    case "update_from_windows":
      return () => projectsApi.updateFromWindows(job.project_id);
    default:
      return null;
  }
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
    const canRetry = RETRYABLE_KINDS.includes(job.kind);
    return (
      <div className="chip chip-failed">
        <code>{job.state.code}</code>
        <span>{job.state.message}</span>
        {job.state.remediation && (
          <div className="muted">→ {job.state.remediation}</div>
        )}
        {canRetry && (
          <button type="button" onClick={() => onRetry(job)}>
            Retry
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
  const [forceRemove, setForceRemove] = useState(false);
  const [removeProblem, setRemoveProblem] = useState<Problem | null>(null);
  const [relocating, setRelocating] = useState<Project | null>(null);
  const [relocatePath, setRelocatePath] = useState("");
  const [relocateProblem, setRelocateProblem] = useState<Problem | null>(null);

  const loadSnapshot = useCallback(() => {
    projectsApi
      .snapshot()
      .then((next) => {
        snapRef.current = next;
        setSnap(next);
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
    const ok = await run(project.id, () =>
      projectsApi.rename(project.id, name),
    );
    if (ok) setEditingId(null);
  }

  function syncToWindows(project: Project) {
    run(project.id, () => projectsApi.syncToWindows(project.id));
  }

  function updateFromWindows(project: Project) {
    run(project.id, () => projectsApi.updateFromWindows(project.id));
  }

  function cancelJobFor(job: Job) {
    run(job.id, () => projectsApi.cancelJob(job.id));
  }

  function retry(job: Job) {
    const fn = retryFn(job);
    if (fn) run(job.project_id, fn);
  }

  function copyPath(path: string) {
    navigator.clipboard?.writeText(path).catch(() => {
      /* best effort — the path is also shown as plain text */
    });
  }

  function openInExplorer(path: string) {
    projectsApi
      .openInExplorer(path)
      .catch((error: unknown) => setProblem(asProblem(error)));
  }

  function openRemoveDialog(project: Project) {
    setRemoving(project);
    setDeleteWorkspace(true);
    setForceRemove(false);
    setRemoveProblem(null);
  }

  function closeRemoveDialog() {
    setRemoving(null);
    setRemoveProblem(null);
    setForceRemove(false);
  }

  async function confirmRemove() {
    if (!removing) return;
    setRemoveProblem(null);
    try {
      await projectsApi.remove(removing.id, deleteWorkspace, forceRemove);
      setRemoving(null);
      setForceRemove(false);
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
                </div>

                <div className="project-row-detail muted">
                  <div>source: {project.source}</div>
                  <div>
                    workspace: <code>{path}</code>
                    <button type="button" onClick={() => copyPath(path)}>
                      Copy
                    </button>
                    <button type="button" onClick={() => openInExplorer(path)}>
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

                <div className="actions">
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
            {removeProblem?.code === "workspace_dirty" && (
              <label>
                <input
                  type="checkbox"
                  checked={forceRemove}
                  onChange={(e) => setForceRemove(e.target.checked)}
                />
                Force (discard uncommitted changes)
              </label>
            )}
            <div className="actions">
              <button type="button" onClick={confirmRemove}>
                {removeProblem?.code === "workspace_dirty"
                  ? "Remove anyway"
                  : "Remove"}
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
