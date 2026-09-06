import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { SandboxLine } from "@/features/sessions/sandbox-line";
import type { SandboxState, Session } from "@/lib/proto";

const session = (sandbox?: SandboxState): Session => ({
  id: "s",
  project_id: "p",
  harness: "claude-code",
  workspace: "/w",
  state: { state: "running" },
  created_at: "1",
  clients: 0,
  sandbox,
});

const sandbox = (over: Partial<SandboxState>): SandboxState => ({
  applied: [],
  unavailable: [],
  degraded: [],
  denied: [],
  ...over,
});

describe("SandboxLine", () => {
  it("shows a chip per applied mechanism with a human label", () => {
    render(
      <SandboxLine
        session={session(
          sandbox({ applied: ["rlimits", "seccomp", "landlock"] }),
        )}
      />,
    );

    expect(screen.getByText("limits")).toBeTruthy();
    expect(screen.getByText("syscall filter")).toBeTruthy();
    expect(screen.getByText("path rules")).toBeTruthy();
  });

  it("shows a warning for an unavailable mechanism and an error for a degraded one", () => {
    render(
      <SandboxLine
        session={session(
          sandbox({
            applied: ["mounts"],
            unavailable: ["landlock"],
            degraded: ["seccomp"],
          }),
        )}
      />,
    );

    expect(screen.getByText(/path rules unavailable/)).toBeTruthy();
    expect(screen.getByText(/syscall filter degraded/)).toBeTruthy();
  });

  it("shows a denials badge that expands the list on click", () => {
    const first = String(Math.floor(Date.now() / 1000) - 300);

    const recent = String(Math.floor(Date.now() / 1000) - 120);

    render(
      <SandboxLine
        session={session(
          sandbox({
            applied: ["seccomp"],
            denied: [
              {
                class: "syscall",
                name: "unshare",
                count: 3,
                first_at: first,
                last_at: recent,
              },
              {
                class: "terminal",
                name: "clipboard",
                count: 1,
                first_at: first,
                last_at: recent,
              },
            ],
          }),
        )}
      />,
    );

    const badge = screen.getByRole("button", { name: /4 denials/ });
    expect(badge).toBeTruthy();

    fireEvent.click(badge);

    const row = screen.getByText(/unshare/);
    expect(row.textContent).toContain("×3");
    expect(row.textContent).toMatch(/min ago|just now/);
    expect(row.textContent).not.toContain("unknown");

    expect(screen.getByText(/clipboard/)).toBeTruthy();
  });

  it("shows a muted no-report line when the posture is unknown", () => {
    render(<SandboxLine session={session()} />);

    expect(screen.getByText("no sandbox report")).toBeTruthy();
  });
});
