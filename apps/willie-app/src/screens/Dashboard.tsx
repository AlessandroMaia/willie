import { useCallback, useEffect, useState } from "react";
import type { EngineStatus, Problem } from "../lib/engine";
import { engine, isProblem, projects } from "../lib/engine";
import type { Part } from "../lib/health";
import { lightFor, overallHealth } from "../lib/health";

const PARTS: { key: Part; label: string }[] = [
  { key: "wsl", label: "WSL" },
  { key: "distro", label: "Distribution" },
  { key: "daemon", label: "Daemon" },
  { key: "doctor", label: "Doctor" },
];

export function Dashboard() {
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [problem, setProblem] = useState<Problem | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [projectCount, setProjectCount] = useState<number | null>(null);

  const report = useCallback((error: unknown) => {
    setProblem(
      isProblem(error)
        ? error
        : { code: "unknown", message: String(error), remediation: "" },
    );
  }, []);

  const refresh = useCallback(() => {
    engine.status().then(setStatus).catch(report);
  }, [report]);

  useEffect(() => {
    refresh();
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    engine.onStatus(setStatus).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [refresh]);

  const daemonState = status?.daemon.state ?? null;

  useEffect(() => {
    /* `state_snapshot` starts the daemon on demand when it is not
     * already running (booting the WSL VM, up to a 60s HELLO
     * timeout) — opening the Dashboard must never be what boots
     * Willie, so the count is only fetched once the daemon is
     * already up, and the effect re-runs as `daemonState` changes. */
    if (daemonState !== "running") {
      setProjectCount(null);
      return;
    }
    let cancelled = false;
    projects
      .snapshot()
      .then((snap) => {
        if (!cancelled) setProjectCount(snap.projects.length);
      })
      .catch(() => {
        /* the count is a nicety; a failed snapshot just hides it */
      });
    return () => {
      cancelled = true;
    };
  }, [daemonState]);

  async function run(name: string, action: () => Promise<unknown>) {
    setBusy(name);
    setProblem(null);
    try {
      await action();
      refresh();
    } catch (error) {
      report(error);
    } finally {
      setBusy(null);
    }
  }

  if (status === null) {
    return <main className="shell">Loading engine status…</main>;
  }

  const overall = overallHealth(status);
  const daemonRunning = status.daemon.state === "running";

  return (
    <main className="dashboard">
      <header>
        <h1>Willie</h1>
        <span
          className={`light light-${overall}`}
          role="img"
          aria-label={`overall ${overall}`}
        />
        <span className="muted">engine v{status.engine_version}</span>
      </header>

      {projectCount !== null && (
        <p className="muted">
          {projectCount} project{projectCount === 1 ? "" : "s"}
        </p>
      )}

      <section className="lights">
        {PARTS.map(({ key, label }) => (
          <div key={key} className="light-row">
            <span className={`light light-${lightFor(key, status)}`} />
            <strong>{label}</strong>
            <span className="muted">{describe(key, status)}</span>
          </div>
        ))}
      </section>

      <section className="actions">
        <button
          type="button"
          disabled={busy !== null || !status.image_available}
          onClick={() => run("install", engine.installDistro)}
        >
          {status.distro?.registered
            ? "Reinstall distribution"
            : "Install distribution"}
        </button>
        <button
          type="button"
          disabled={busy !== null || !status.distro?.registered}
          onClick={() =>
            run(
              "daemon",
              daemonRunning ? engine.stopDaemon : engine.startDaemon,
            )
          }
        >
          {daemonRunning ? "Stop daemon" : "Start daemon"}
        </button>
        <button
          type="button"
          disabled={busy !== null || !status.distro?.registered}
          onClick={() => run("doctor", engine.doctor)}
        >
          Run doctor
        </button>
        {busy && <span className="muted">{busy}…</span>}
      </section>

      {problem && (
        <section className="problem" role="alert">
          <strong>{problem.code}</strong> — {problem.message}
          {problem.remediation && (
            <div className="muted">→ {problem.remediation}</div>
          )}
        </section>
      )}

      {status.doctor && (
        <section className="doctor">
          <h2>Doctor</h2>
          <ul>
            {status.doctor.checks.map((c) => (
              <li key={c.name} className={`check check-${c.status}`}>
                <code>[{c.status}]</code> <strong>{c.name}</strong>{" "}
                <span className="muted">{c.detail}</span>
                {c.status === "fail" && c.remediation && (
                  <div className="muted">→ {c.remediation}</div>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}
    </main>
  );
}

function describe(part: Part, s: EngineStatus): string {
  switch (part) {
    case "wsl":
      if (!s.wsl.installed) return "not installed";
      /* wsl.exe ran but reported no version: an update, not an install. */
      if (s.wsl.version === null)
        return "installed, version unknown — update WSL";
      return `${s.wsl.version} (minimum ${s.wsl.minimum})`;
    case "distro":
      if (s.distro_error) return s.distro_error.message;
      if (!s.distro?.registered) {
        return s.image_available
          ? "not registered — image available"
          : "not registered — no image found";
      }
      return s.distro.running ? "registered, running" : "registered, stopped";
    case "daemon":
      if (s.daemon.state === "running") {
        return `v${s.daemon.willie_version} · image ${s.daemon.image_version ?? "?"}`;
      }
      if (s.daemon.state === "failed") {
        return `${s.daemon.code}: ${s.daemon.message}`;
      }
      return "stopped";
    case "doctor":
      return s.doctor ? `${s.doctor.checks.length} checks` : "not run yet";
  }
}
