import type { Tone } from "@/components/tone";
import type { SessionState } from "@/lib/proto";

/** What a session state says on its badge and the tone it takes.
 * Presentation, so it lives beside the badge rather than in lib/. */
export function badgeFor(state: SessionState): { label: string; tone: Tone } {
  switch (state.state) {
    case "creating":
      return { label: "creating", tone: "pending" };
    case "running":
      return { label: "running", tone: "ok" };
    case "stopping":
      return { label: "stopping", tone: "pending" };
    case "exited": {
      if (state.code == null && state.signal != null) {
        return { label: `exited (signal ${state.signal})`, tone: "error" };
      }
      const code = state.code ?? 0;
      return { label: `exited ${code}`, tone: code === 0 ? "muted" : "error" };
    }
    case "failed":
      return { label: `failed: ${state.code}`, tone: "error" };
  }
}

/* Until Task 9 the badge still renders with the old chip classes so
 * nothing changes on screen while the mapping moves. */
const CHIP_CLASS: Record<Tone, string> = {
  ok: "chip chip-ready",
  warning: "chip chip-busy",
  error: "chip chip-failed",
  pending: "chip chip-busy",
  muted: "chip chip-muted",
};

interface SessionStateBadgeProps {
  state: SessionState;
}

/* A failed state carries its own code, message and remediation, so it
 * renders stacked; every other state is a single-line pill. */
export function SessionStateBadge({ state }: SessionStateBadgeProps) {
  const { label, tone } = badgeFor(state);

  if (state.state === "failed") {
    return (
      <div className={CHIP_CLASS[tone]}>
        <code>{label}</code>
        <span>{state.message}</span>
        {state.remediation && (
          <div className="muted">→ {state.remediation}</div>
        )}
      </div>
    );
  }

  return (
    <span className={CHIP_CLASS[tone]}>
      {tone === "pending" && <span className="spinner" aria-hidden="true" />}
      {label}
    </span>
  );
}
