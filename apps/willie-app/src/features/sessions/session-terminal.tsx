import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef, useState } from "react";
import { ProblemAlert } from "@/components/problem-alert";
import type { Problem } from "@/lib/ipc";
import { onSessionOutput, sessionTerminal } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";

interface SessionTerminalProps {
  id: string;
  title: string;
}

/* Renders one session's live terminal: a byte-stream render channel, not
 * session state — the Sessions screen still derives live/exited truth
 * from the snapshot. Keyed by `id` at the call site so switching
 * sessions remounts this component instead of re-running a branching
 * effect. */
export function SessionTerminal({ id, title }: SessionTerminalProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [problem, setProblem] = useState<Problem | null>(null);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const term = new Terminal({ convertEol: false });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(container);

    let cancelled = false;
    let unlisten: (() => void) | undefined;

    function reportResize() {
      fit.fit();
      sessionTerminal.resize(id, term.rows, term.cols).catch(() => {
        /* Best-effort: a resize racing session teardown is not a user-
         * facing failure — the close path already surfaces that. */
      });
    }

    const observer = new ResizeObserver(() => reportResize());
    observer.observe(container);

    onSessionOutput((out) => {
      if (cancelled) return;
      if (out.id === id) term.write(new Uint8Array(out.chunk));
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch((error: unknown) => {
        if (!cancelled) setProblem(asProblem(error));
      });

    term.onData((data) => {
      sessionTerminal.input(id, data).catch(() => {
        /* Best-effort: input racing session teardown is not surfaced
         * here — the terminal simply stops echoing. */
      });
    });

    sessionTerminal
      .open(id)
      .then(() => {
        if (cancelled) return;
        reportResize();
      })
      .catch((error: unknown) => {
        if (!cancelled) setProblem(asProblem(error));
      });

    return () => {
      cancelled = true;
      unlisten?.();
      observer.disconnect();
      sessionTerminal.close(id).catch(() => {
        /* Best-effort: the session may already be gone. */
      });
      term.dispose();
    };
  }, [id]);

  return (
    <div className="session-terminal-body">
      {problem && <ProblemAlert problem={problem} />}
      <div className="terminal-surface" ref={containerRef} title={title} />
    </div>
  );
}
