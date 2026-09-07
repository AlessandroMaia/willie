import { useCallback, useEffect, useRef, useState } from "react";
import { FailureChip } from "@/components/failure-chip";
import { ProblemAlert } from "@/components/problem-alert";
import { StatusDot } from "@/components/status-dot";
import { type Tone, toneForHealth } from "@/components/tone";
import { Button } from "@/components/ui/button";
import { ButtonGroup } from "@/components/ui/button-group";
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemMedia,
  ItemTitle,
} from "@/components/ui/item";
import { Spinner } from "@/components/ui/spinner";
import {
  LogonFixAction,
  offersLogonFix,
} from "@/features/health/logon-fix-action";
import { healthFor, overallHealth, type Part } from "@/lib/domain/health";
import { lastLogLine, latestInstallJob } from "@/lib/domain/jobs";
import type { EngineStatus, Problem } from "@/lib/ipc";
import { engine, tools } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import { useEngineStatus } from "@/store/use-engine-status";
import { useSnapshot } from "@/store/use-snapshot";

const PARTS: { key: Part; label: string }[] = [
  { key: "wsl", label: "WSL" },
  { key: "distro", label: "Distribution" },
  { key: "daemon", label: "Daemon" },
  { key: "doctor", label: "Doctor" },
];

const CHECK_TONE: Record<"ok" | "fail" | "skip", Tone> = {
  ok: "ok",
  fail: "error",
  skip: "muted",
};

export function DashboardScreen() {
  const { status, problem: statusProblem, refresh } = useEngineStatus();
  const [problem, setProblem] = useState<Problem | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const handledInstallJobRef = useRef<string | null>(null);

  const report = useCallback((error: unknown) => {
    setProblem(asProblem(error));
  }, []);

  const daemonState = status?.daemon.state ?? null;

  /* `state_snapshot` starts the daemon on demand when it is not
   * already running (booting the WSL VM, up to a 60s HELLO timeout) —
   * opening the Dashboard must never be what boots Willie, so the
   * store only acquires the snapshot once the daemon is already up,
   * and releases it the moment `daemonState` leaves "running". A
   * failed snapshot is a nicety here, not a page failure, and never a
   * banner: before the first success the count and install job stay
   * unset, and after that the store keeps the last one it had, so a
   * later failure does not blank out either. */
  const store = useSnapshot(daemonState === "running");
  const snap = store.snapshot;

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
      engine
        .doctor()
        .then(() => refresh())
        .catch(report);
    }
  }, [installJob?.id, installJob?.state.state, refresh, report]);

  async function run(name: string, action: () => Promise<unknown>) {
    setBusy(name);
    setProblem(null);
    try {
      await action();
      await refresh();
    } catch (error) {
      report(error);
    } finally {
      setBusy(null);
    }
  }

  if (status === null) {
    return (
      <div className="flex items-center gap-2 text-muted-foreground text-sm">
        <Spinner /> Loading engine status…
      </div>
    );
  }

  const overall = overallHealth(status);
  const daemonRunning = status.daemon.state === "running";
  const shown = problem ?? statusProblem;

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-6">
      <header className="flex items-center gap-3">
        <h1 className="font-semibold text-lg">Dashboard</h1>
        <StatusDot tone={toneForHealth(overall)} label={`overall ${overall}`} />
        <span className="font-mono text-muted-foreground text-xs">
          engine v{status.engine_version}
        </span>
        {projectCount !== null && (
          <span className="text-muted-foreground text-sm">
            {projectCount} project{projectCount === 1 ? "" : "s"}
          </span>
        )}
      </header>

      <ItemGroup className="gap-1">
        {PARTS.map(({ key, label }) => (
          <Item key={key} size="sm" variant="muted">
            <ItemMedia>
              <StatusDot tone={toneForHealth(healthFor(key, status))} />
            </ItemMedia>
            <ItemContent>
              <ItemTitle>{label}</ItemTitle>
              <ItemDescription>{describe(key, status)}</ItemDescription>
            </ItemContent>
          </Item>
        ))}
      </ItemGroup>

      <ButtonGroup>
        <Button
          variant="outline"
          disabled={busy !== null || !status.image_available}
          onClick={() => run("install", engine.installDistro)}
        >
          {status.distro?.registered
            ? "Reinstall distribution"
            : "Install distribution"}
        </Button>
        <Button
          variant="outline"
          disabled={busy !== null || !status.distro?.registered}
          onClick={() =>
            run(
              "daemon",
              daemonRunning ? engine.stopDaemon : engine.startDaemon,
            )
          }
        >
          {daemonRunning ? "Stop daemon" : "Start daemon"}
        </Button>
        <Button
          variant="outline"
          disabled={busy !== null || !status.distro?.registered}
          onClick={() => run("doctor", engine.doctor)}
        >
          Run doctor
        </Button>
      </ButtonGroup>
      {busy && (
        <div className="flex items-center gap-2 text-muted-foreground text-sm">
          <Spinner /> {busy}…
        </div>
      )}

      {shown && (
        <ProblemAlert
          problem={shown}
          action={offersLogonFix(shown) ? <LogonFixAction /> : undefined}
        />
      )}

      {status.doctor && (
        <section className="flex flex-col gap-2">
          <h2 className="font-medium text-muted-foreground text-sm">Doctor</h2>
          <ItemGroup className="gap-1">
            {status.doctor.checks.map((c) => {
              const isHarnessCheck = c.name === "Claude Code";
              const installing = installJob?.state.state === "running";
              return (
                <Item key={c.name} size="sm" variant="outline">
                  <ItemMedia>
                    <StatusDot tone={CHECK_TONE[c.status]} label={c.status} />
                  </ItemMedia>
                  <ItemContent>
                    <ItemTitle>{c.name}</ItemTitle>
                    <ItemDescription>{c.detail}</ItemDescription>
                    {c.status === "fail" && c.remediation && (
                      <ItemDescription>{c.remediation}</ItemDescription>
                    )}
                    {isHarnessCheck &&
                      c.status === "fail" &&
                      installJob &&
                      installing && (
                        <ItemDescription className="font-mono">
                          {lastLogLine(installJob)}
                        </ItemDescription>
                      )}
                    {isHarnessCheck &&
                      c.status === "fail" &&
                      installJob?.state.state === "failed" && (
                        <FailureChip
                          code={installJob.state.code}
                          message={installJob.state.message}
                          remediation={installJob.state.remediation}
                        />
                      )}
                  </ItemContent>
                  {isHarnessCheck && c.status === "fail" && (
                    <ItemActions>
                      <Button
                        size="sm"
                        disabled={busy !== null || installing}
                        onClick={() =>
                          run("install-harness", () =>
                            tools.install("claude-code"),
                          )
                        }
                      >
                        {installing && <Spinner />} Install
                      </Button>
                    </ItemActions>
                  )}
                </Item>
              );
            })}
          </ItemGroup>
        </section>
      )}
    </div>
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
