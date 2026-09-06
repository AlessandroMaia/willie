import { useState } from "react";

import { relativeTime } from "@/components/relative-time";
import { StatusBadge } from "@/components/status-badge";
import { denials, sandboxOf, sandboxPosture } from "@/lib/domain/sessions";
import type { Session } from "@/lib/proto";

/**
 * The human label for a sandbox mechanism. An unknown name shows as-is,
 * so a mechanism added in a later phase is never hidden. Presentation, so
 * the labels live in the feature rather than in `lib/`.
 */
export function mechanismLabel(name: string): string {
  switch (name) {
    case "rlimits":
      return "limits";
    case "seccomp":
      return "syscall filter";
    case "landlock":
      return "path rules";
    default:
      return name;
  }
}

interface SandboxLineProps {
  session: Session;
}

/**
 * A single line of a session row: a chip per applied mechanism, a warning
 * chip for anything the kernel could not offer, an error chip for anything
 * that degraded, and a warning badge that expands the denials with their
 * counts. A session with no report shows "no sandbox report" rather than a
 * guess. Colour flows only through `StatusBadge`'s tone.
 */
export function SandboxLine({ session }: SandboxLineProps) {
  const [open, setOpen] = useState(false);

  const posture = sandboxPosture(session);

  if (posture === "unknown") {
    return <StatusBadge tone="muted">no sandbox report</StatusBadge>;
  }

  const sb = sandboxOf(session);

  const { items, total } = denials(session);

  return (
    <div className="flex flex-wrap items-center gap-1">
      {sb.applied.map((m) => (
        <StatusBadge key={`applied-${m}`} tone="muted">
          {mechanismLabel(m)}
        </StatusBadge>
      ))}

      {sb.unavailable.map((m) => (
        <StatusBadge key={`unavailable-${m}`} tone="warning">
          {mechanismLabel(m)} unavailable
        </StatusBadge>
      ))}

      {sb.degraded.map((m) => (
        <StatusBadge key={`degraded-${m}`} tone="error">
          {mechanismLabel(m)} degraded
        </StatusBadge>
      ))}

      {total > 0 && (
        <button
          type="button"
          className="cursor-pointer"
          aria-expanded={open}
          onClick={() => setOpen((o) => !o)}
        >
          <StatusBadge tone="warning">
            {total} denial{total === 1 ? "" : "s"}
          </StatusBadge>
        </button>
      )}

      {open && total > 0 && (
        <ul className="flex max-h-32 w-full flex-col gap-0.5 overflow-y-auto text-muted-foreground text-xs">
          {items.map((d) => (
            <li key={`${d.class}-${d.name}`}>
              {d.name} · {d.class} · ×{d.count} · {relativeTime(d.last_at)}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
