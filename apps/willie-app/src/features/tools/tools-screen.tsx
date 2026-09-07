import { useCallback, useEffect, useRef, useState } from "react";
import { ProblemAlert } from "@/components/problem-alert";
import { StatusBadge } from "@/components/status-badge";
import { Button } from "@/components/ui/button";
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemTitle,
} from "@/components/ui/item";
import { Spinner } from "@/components/ui/spinner";
import { lastLogLine, latestToolJob } from "@/lib/domain/jobs";
import type { Problem } from "@/lib/ipc";
import { tools } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import type { ToolStatus } from "@/lib/proto";
import { useSnapshot } from "@/store/use-snapshot";

/* The daemon allows only one tool job at a time (a second `install` or
 * `update` is refused outright), so the newest install/update job
 * always belongs to whichever tool the running action is for — there
 * is no per-tool job id to match against, unlike a project job. Today
 * the registry holds exactly one managed tool, so this never shows the
 * wrong row's progress; a second managed tool would need the job to
 * carry which tool it is for. */
export function ToolsScreen() {
  const store = useSnapshot();
  const jobs = store.snapshot?.jobs ?? [];
  const [toolList, setToolList] = useState<ToolStatus[]>([]);
  const [problem, setProblem] = useState<Problem | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const handledJobRef = useRef<string | null>(null);

  const load = useCallback(() => {
    tools
      .list()
      .then((result) => setToolList(result.tools))
      .catch((error: unknown) => setProblem(asProblem(error)));
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const job = latestToolJob(jobs);
  const running = job?.state.state === "running";

  useEffect(() => {
    /* Fires once per job: a `useRef` (not state) remembers the last
     * job id this effect acted on, since recording it in state would
     * itself retrigger the effect — same pattern as the Dashboard's
     * install-job tracking. */
    if (
      job?.state.state &&
      job.state.state !== "running" &&
      handledJobRef.current !== job.id
    ) {
      handledJobRef.current = job.id;
      load();
    }
  }, [job?.id, job?.state.state, load]);

  async function run(id: string, action: () => Promise<unknown>) {
    setBusyId(id);
    setProblem(null);
    try {
      await action();
    } catch (error) {
      setProblem(asProblem(error));
    } finally {
      setBusyId(null);
    }
  }

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-6">
      <header>
        <h1 className="font-semibold text-lg">Tools</h1>
      </header>

      {problem && <ProblemAlert problem={problem} />}

      <ItemGroup className="gap-1">
        {toolList.map((status) => {
          const outdatedRecord =
            status.recorded_version !== undefined &&
            status.recorded_version !== status.version;
          return (
            <Item key={status.id} variant="outline">
              <ItemContent>
                <ItemTitle>{status.name}</ItemTitle>
                {status.installed ? (
                  <StatusBadge tone="ok">
                    installed v{status.version}
                  </StatusBadge>
                ) : (
                  <StatusBadge tone="muted">not installed</StatusBadge>
                )}
                {outdatedRecord && (
                  <ItemDescription>updated outside Willie</ItemDescription>
                )}
                {running && (
                  <ItemDescription className="flex items-center gap-2 font-mono">
                    <Spinner /> {lastLogLine(job)}
                  </ItemDescription>
                )}
              </ItemContent>
              <ItemActions>
                {status.installed ? (
                  <Button
                    size="sm"
                    disabled={busyId !== null || running}
                    onClick={() =>
                      run(status.id, () => tools.update(status.id))
                    }
                  >
                    {busyId === status.id && <Spinner />} Update
                  </Button>
                ) : (
                  <Button
                    size="sm"
                    disabled={busyId !== null || running}
                    onClick={() =>
                      run(status.id, () => tools.install(status.id))
                    }
                  >
                    {busyId === status.id && <Spinner />} Install
                  </Button>
                )}
              </ItemActions>
            </Item>
          );
        })}
      </ItemGroup>
    </div>
  );
}
