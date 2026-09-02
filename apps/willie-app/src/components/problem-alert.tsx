import { CircleAlertIcon, InfoIcon } from "lucide-react";
import type { ReactNode } from "react";
import {
  Alert,
  AlertAction,
  AlertDescription,
  AlertTitle,
} from "@/components/ui/alert";
import type { Problem } from "@/lib/ipc";

interface ProblemAlertProps {
  problem: Problem;
  /** `error` is a failure the user must act on; `notice` is the shape
   * used when an action succeeded but something beside it did not —
   * a live session whose terminal tab never opened. */
  tone?: "error" | "notice";
  /** One control that belongs to this failure, shown in the alert's own
   * action corner. This component never learns which failure it is
   * looking at: the caller decides what a given code deserves. */
  action?: ReactNode;
}

export function ProblemAlert({
  problem,
  tone = "error",
  action,
}: ProblemAlertProps) {
  if (tone === "notice") {
    return (
      <Alert role="status">
        <InfoIcon />
        <AlertTitle>{problem.message}</AlertTitle>
        {problem.remediation && (
          <AlertDescription>{problem.remediation}</AlertDescription>
        )}
        {action && <AlertAction>{action}</AlertAction>}
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
      {action && <AlertAction>{action}</AlertAction>}
    </Alert>
  );
}
