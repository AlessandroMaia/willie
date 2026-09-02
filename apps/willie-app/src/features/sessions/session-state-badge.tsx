import { FailureChip } from "@/components/failure-chip";
import { StatusBadge } from "@/components/status-badge";
import type { Tone } from "@/components/tone";
import { Spinner } from "@/components/ui/spinner";
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

interface SessionStateBadgeProps {
  state: SessionState;
}

/* A failed state carries its own code, message and remediation, so it
 * renders stacked; every other state is a single-line pill. */
export function SessionStateBadge({ state }: SessionStateBadgeProps) {
  const { label, tone } = badgeFor(state);

  if (state.state === "failed") {
    return (
      <FailureChip
        code={label}
        message={state.message}
        remediation={state.remediation}
      />
    );
  }

  return (
    <StatusBadge tone={tone}>
      {tone === "pending" && <Spinner className="size-3" />}
      {label}
    </StatusBadge>
  );
}
