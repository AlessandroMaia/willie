import type { Problem } from "@/lib/ipc";

/** Every rejection crossing the bridge is either a daemon `Problem` or
 * something unexpected; the screens render one shape, so an unexpected
 * error is given the same one rather than a second code path. */
export function asProblem(error: unknown): Problem {
  return isProblem(error)
    ? error
    : { code: "unknown", message: String(error), remediation: "" };
}

function isProblem(value: unknown): value is Problem {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    "message" in value
  );
}
