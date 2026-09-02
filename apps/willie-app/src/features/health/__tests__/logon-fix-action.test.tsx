import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  LogonFixAction,
  offersLogonFix,
} from "@/features/health/logon-fix-action";
import type { Problem } from "@/lib/ipc";

const SCRIPT = "# Run in an elevated PowerShell (Run as administrator).\n";

/* The bridge is the only module a test fakes: it is the only I/O. */
const ipc = vi.hoisted(() => ({
  engine: { logonFixScript: vi.fn() },
}));

vi.mock("@/lib/ipc", () => ipc);

const problem = (code: string): Problem => ({
  code,
  message: "daemon exited with code 1",
  remediation: "an administrator must grant it",
});

beforeEach(() => {
  vi.clearAllMocks();
  ipc.engine.logonFixScript.mockResolvedValue(SCRIPT);
});

describe("offersLogonFix", () => {
  it("recognises the one problem an administrator has to clear", () => {
    expect(offersLogonFix(problem("service_logon_right_missing"))).toBe(true);
  });

  it("leaves every other failure alone", () => {
    expect(offersLogonFix(problem("daemon_exited"))).toBe(false);
    expect(offersLogonFix(problem("wsl_command_failed"))).toBe(false);
    expect(offersLogonFix(null)).toBe(false);
  });
});

describe("LogonFixAction", () => {
  it("copies the commands the engine owns", async () => {
    const user = userEvent.setup();
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
    render(<LogonFixAction />);

    await user.click(screen.getByRole("button"));

    await vi.waitFor(() => expect(writeText).toHaveBeenCalledWith(SCRIPT));
    expect(ipc.engine.logonFixScript).toHaveBeenCalledTimes(1);
  });

  it("says what went wrong when the clipboard refuses", async () => {
    const user = userEvent.setup();
    Object.defineProperty(navigator, "clipboard", {
      value: {
        writeText: vi.fn(async () => {
          throw new Error("denied");
        }),
      },
      configurable: true,
    });
    render(<LogonFixAction />);

    await user.click(screen.getByRole("button"));

    /* The button stays usable: a refused clipboard is not a dead end. */
    await vi.waitFor(() =>
      expect((screen.getByRole("button") as HTMLButtonElement).disabled).toBe(
        false,
      ),
    );
  });
});
