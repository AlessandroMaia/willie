import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef, useState } from "react";
import type { Problem } from "../lib/engine";
import { isProblem, onSessionOutput, sessionTerminal } from "../lib/engine";

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
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
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
        if (!cancelled) {
          setProblem(
            isProblem(error)
              ? error
              : { code: "unknown", message: String(error), remediation: "" },
          );
        }
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
      {problem && (
        <div className="problem" role="alert">
          <strong>{problem.code}</strong> — {problem.message}
          {problem.remediation && (
            <div className="muted">→ {problem.remediation}</div>
          )}
        </div>
      )}
      <div className="terminal-surface" ref={containerRef} title={title} />
    </div>
  );
}
