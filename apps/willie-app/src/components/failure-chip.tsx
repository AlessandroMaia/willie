import type { ReactNode } from "react";
import { TONE_SURFACE } from "@/components/tone";

interface FailureChipProps {
  code: string;
  message: string;
  remediation?: string | null;
  /** `error` (the default) is a genuine failure; `warning` is a policy
   * problem that leaves the workspace itself fine — a project whose
   * `[sandbox]` table could not be read, for instance. */
  tone?: "error" | "warning";
  /** Actions that belong to this failure, a Retry for instance. */
  children?: ReactNode;
}

/** A failed state stacked into code, message and remediation. Projects,
 * jobs and sessions all carry that shape, so they all render this. */
export function FailureChip({
  code,
  message,
  remediation,
  tone = "error",
  children,
}: FailureChipProps) {
  return (
    <div
      className={`flex flex-col gap-1 rounded-lg px-2.5 py-1.5 text-xs ${TONE_SURFACE[tone === "warning" ? "warning" : "error"]}`}
    >
      <code className="font-mono">{code}</code>
      <span>{message}</span>
      {remediation && (
        <span className="text-muted-foreground">{remediation}</span>
      )}
      {children && <div className="flex gap-2 pt-1">{children}</div>}
    </div>
  );
}
