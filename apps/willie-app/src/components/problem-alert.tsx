import type { Problem } from "@/lib/ipc";

interface ProblemAlertProps {
  problem: Problem;
  /** `error` is a failure the user must act on; `notice` is the
   * dismissable shape used when an action succeeded but something
   * beside it did not — a live session whose terminal tab never
   * opened. */
  tone?: "error" | "notice";
}

export function ProblemAlert({ problem, tone = "error" }: ProblemAlertProps) {
  if (tone === "notice") {
    return (
      <div className="notice" role="status">
        <span>{problem.message}</span>
        {problem.remediation && (
          <div className="muted">→ {problem.remediation}</div>
        )}
      </div>
    );
  }
  return (
    <div className="problem" role="alert">
      <strong>{problem.code}</strong> — {problem.message}
      {problem.remediation && (
        <div className="muted">→ {problem.remediation}</div>
      )}
    </div>
  );
}
