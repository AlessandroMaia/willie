import type { ReactNode } from "react";

interface FailureChipProps {
  code: string;
  message: string;
  remediation?: string | null;
  /** Actions that belong to this failure, a Retry for instance. */
  children?: ReactNode;
}

/** A failed state stacked into code, message and remediation. Projects,
 * jobs and sessions all carry that shape, so they all render this. */
export function FailureChip({
  code,
  message,
  remediation,
  children,
}: FailureChipProps) {
  return (
    <div className="flex flex-col gap-1 rounded-lg bg-destructive/10 px-2.5 py-1.5 text-destructive text-xs">
      <code className="font-mono">{code}</code>
      <span>{message}</span>
      {remediation && (
        <span className="text-muted-foreground">{remediation}</span>
      )}
      {children && <div className="flex gap-2 pt-1">{children}</div>}
    </div>
  );
}
