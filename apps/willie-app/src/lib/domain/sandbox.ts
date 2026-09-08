import type { SandboxState } from "@/lib/proto";

/**
 * The applied sandbox mechanisms, joined into one line. Empty when
 * none were reported yet — a session still `creating`, or one recorded
 * before sandbox reporting existed.
 */
export function postureLine(sandbox: SandboxState): string {
  return sandbox.applied.join(" · ");
}

/** How many denials the sandbox recorded in total, across every kind. */
export function deniedCount(sandbox: SandboxState): number {
  return sandbox.denied.reduce((sum, d) => sum + d.count, 0);
}
