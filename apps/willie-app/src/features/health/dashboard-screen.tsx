import { useCallback, useEffect, useRef, useState } from "react";
import { useSnapshot } from "@/app/store";
import type { Part } from "@/lib/domain/health";
import { lightFor, overallHealth } from "@/lib/domain/health";
import { latestInstallJob } from "@/lib/domain/jobs";
import type { EngineStatus, Problem } from "@/lib/ipc";
import { engine, tools } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import type { Job } from "@/lib/proto";

const PARTS: { key: Part; label: string }[] = [
  { key: "wsl", label: "WSL" },
  { key: "distro", label: "Distribution" },
  { key: "daemon", label: "Daemon" },
  { key: "doctor", label: "Doctor" },
];

export function DashboardScreen() {
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [problem, setProblem] = useState<Problem | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const handledInstallJobRef = useRef<string | null>(null);

  const report = useCallback((error: unknown) => {
    setProblem(asProblem(error));
  }, []);

  const refresh = useCallback(() => {
    engine.status().then(setStatus).catch(report);
  }, [report]);

  useEffect(() => {
    refresh();
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    engine
      .onStatus(setStatus)
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(report);
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [refresh, report]);

  const daemonState = status?.daemon.state ?? null;

  /* `state_snapshot` starts the daemon on demand when it is not
   * already running (booting the WSL VM, up to a 60s HELLO timeout) —
   * opening the Dashboard must never be what boots Willie, so the
   * store only acquires the snapshot once the daemon is already up,
   * and releases it the moment `daemonState` leaves "running". A
   * failed snapshot is a nicety here, not a page failure: the project
   * count and install job just disappear, with no banner. */
  const store = useSnapshot(daemonState === "running");
  const snap = store.status === "ready" ? store.snapshot : null;

  const projectCount = snap?.projects.length ?? null;
  const installJob = latestInstallJob(snap?.jobs ?? []);

  useEffect(() => {
    /* Fires once per job: a `useRef` (not state) remembers the last
     * job id this effect acted on, since recording it in state would
     * itself retrigger the effect. */
    if (
      installJob?.state.state === "done" &&
      handledInstallJobRef.current !== installJob.id
    ) {
      handledInstallJobRef.current = installJob.id;
      engine.doctor().then(refresh).catch(report);
    }
  }, [installJob?.id, installJob?.state.state, refresh, report]);

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
            {status.doctor.checks.map((c) => {
              const isHarnessCheck = c.name === "Claude Code";
              return (
                <li key={c.name} className={`check check-${c.status}`}>
                  <code>[{c.status}]</code> <strong>{c.name}</strong>{" "}
                  <span className="muted">{c.detail}</span>
                  {c.status === "fail" && c.remediation && (
                    <div className="muted">→ {c.remediation}</div>
                  )}
                  {isHarnessCheck && c.status === "fail" && (
                    <div className="actions">
                      <button
                        type="button"
                        disabled={
                          busy !== null || installJob?.state.state === "running"
                        }
                        onClick={() =>
                          run("install-harness", () =>
                            tools.install("claude-code"),
                          )
                        }
                      >
                        Install
                      </button>
                    </div>
                  )}
                  {isHarnessCheck &&
                    c.status === "fail" &&
                    installJob?.state.state === "running" && (
                      <div className="muted">{lastLogLine(installJob)}</div>
                    )}
                  {isHarnessCheck &&
                    c.status === "fail" &&
                    installJob?.state.state === "failed" && (
                      <div>
                        <code>{installJob.state.code}</code> —{" "}
                        {installJob.state.message}
                        {installJob.state.remediation && (
                          <div className="muted">
                            → {installJob.state.remediation}
                          </div>
                        )}
                      </div>
                    )}
                </li>
              );
            })}
          </ul>
        </section>
      )}
    </main>
  );
}

/* `log_tail` is a multi-line buffer the daemon keeps appending to as
 * the job runs, often with trailing blank lines; only the last real
 * line is worth showing beside the check. */
function lastLogLine(job: Job): string {
  const lines = job.log_tail.split("\n");
  while (lines.length > 0 && lines[lines.length - 1]?.trim() === "") {
    lines.pop();
  }
  return lines[lines.length - 1] ?? "";
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
