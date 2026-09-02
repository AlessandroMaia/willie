import { ClipboardIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { toast } from "@/components/ui/toast";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { Problem } from "@/lib/ipc";
import { engine } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";

/* The contract is the code, never the message: see the engine problem
 * codes in docs/PROTOCOL.md. */
const SERVICE_LOGON_RIGHT_MISSING = "service_logon_right_missing";

/** True for the one failure whose remedy is a script somebody with
 * administrator rights runs, rather than anything the user can click. */
export function offersLogonFix(problem: Problem | null): boolean {
  return problem?.code === SERVICE_LOGON_RIGHT_MISSING;
}

/**
 * Puts the commands that grant the right on the clipboard, so the user
 * can run them in an elevated prompt or hand them to whoever can. The
 * engine owns the text: it is Windows knowledge, and it has to stay
 * beside the remediation it implements.
 */
export function LogonFixAction() {
  async function copy() {
    try {
      const clipboard = navigator.clipboard;
      if (!clipboard) {
        throw new Error("this window has no clipboard access");
      }

      await clipboard.writeText(await engine.logonFixScript());
      toast.add({
        type: "success",
        title: "Commands copied",
        description: "Run them in an elevated PowerShell, then sign in again",
      });
    } catch (error) {
      toast.add({
        type: "error",
        title: "Could not copy the commands",
        description: asProblem(error).message,
      });
    }
  }

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant="outline"
            size="icon-xs"
            aria-label="Copy the commands an administrator must run"
            onClick={() => void copy()}
          />
        }
      >
        <ClipboardIcon />
      </TooltipTrigger>
      <TooltipContent>Copy the commands for an administrator</TooltipContent>
    </Tooltip>
  );
}
