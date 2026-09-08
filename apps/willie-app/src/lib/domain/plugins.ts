import type { Problem } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";

/**
 * Whether a plugin RPC (`profile.list`, `profile.check`, …) rejected
 * because the plugin is not enabled for the project it was scoped to —
 * the fresh-install "first click" state a screen should offer to fix,
 * not report as a failure. The engine flattens every daemon `RpcError`
 * to `code: "daemon_error"`, which puts the real code only in the
 * message until that flattening is fixed, so both shapes count.
 */
export function isPluginDisabled(error: unknown): boolean {
  const problem: Problem = asProblem(error);

  if (problem.code === "plugin_disabled") return true;

  return (
    problem.code === "daemon_error" &&
    problem.message.includes("plugin_disabled")
  );
}
