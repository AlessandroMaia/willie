import { CircleAlertIcon, InfoIcon } from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import type { Problem } from "@/lib/ipc";

interface ProblemAlertProps {
  problem: Problem;
  /** `error` is a failure the user must act on; `notice` is the shape
   * used when an action succeeded but something beside it did not —
   * a live session whose terminal tab never opened. */
  tone?: "error" | "notice";
}

export function ProblemAlert({ problem, tone = "error" }: ProblemAlertProps) {
  if (tone === "notice") {
    return (
      <Alert role="status">
        <InfoIcon />
        <AlertTitle>{problem.message}</AlertTitle>
        {problem.remediation && (
          <AlertDescription>{problem.remediation}</AlertDescription>
        )}
      </Alert>
    );
  }

  return (
    <Alert variant="destructive" role="alert">
      <CircleAlertIcon />
      <AlertTitle>
        <code className="font-mono">{problem.code}</code> — {problem.message}
      </AlertTitle>
      {problem.remediation && (
        <AlertDescription>{problem.remediation}</AlertDescription>
      )}
    </Alert>
  );
}
