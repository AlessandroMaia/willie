import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { SandboxDialog } from "@/features/projects/sandbox-dialog";
import type { CapabilityInfo, Project, SandboxProfile } from "@/lib/proto";

/* Nothing overridden: every capability comes from the harness. */
const inherited: SandboxProfile = { extra_paths: [] };

/* The catalogue the Rust side owns, trimmed to what these tests read.
 * The dialog takes it as a prop, so no module is faked here. */
const catalogue = (): CapabilityInfo[] => [
  {
    capability: "project_rw",
    display_name: "project.rw",
    consequence: "the session edits the project, which is why it exists",
    implemented: true,
  },
  {
    capability: "agent_state",
    display_name: "agent.state",
    consequence: "anything the agent runs can use the harness's login",
    implemented: true,
  },
  {
    capability: "ssh",
    display_name: "ssh",
    consequence: "the agent can use the user's ssh keys and agent socket",
    implemented: false,
  },
];

/* Fill the remaining fields from the fixture the neighbouring projects
 * tests already use, so one project shape is described in one place. */
const project = (): Project => ({
  id: "proj_1",
  name: "willie",
  slug: "willie",
  source: "C:\\github\\willie",
  workspace: "/home/willie/projects/willie",
  branch: "main",
  state: { state: "ready" },
  source_present: true,
  created_at: "1",
  sandbox: inherited,
});

describe("SandboxDialog", () => {
  it("shows each capability with the sentence that says what it costs", () => {
    render(
      <SandboxDialog
        project={project()}
        catalogue={catalogue()}
        problem={null}
        onSave={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    expect(screen.getByText(/anything the agent runs can use/)).toBeDefined();
  });

  it("saves only what the user changed", async () => {
    const user = userEvent.setup();
    const onSave = vi.fn();
    render(
      <SandboxDialog
        project={project()}
        catalogue={catalogue()}
        problem={null}
        onSave={onSave}
        onCancel={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("checkbox", { name: /agent\.state/ }));
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(onSave).toHaveBeenCalledWith(
      expect.objectContaining({ agent_state: false }),
    );
    expect(onSave.mock.calls[0]?.[0]).not.toHaveProperty("tools_ro");
  });

  it("shows a deferred capability as unavailable, not as off", async () => {
    const user = userEvent.setup();
    const onSave = vi.fn();
    render(
      <SandboxDialog
        project={project()}
        catalogue={catalogue()}
        problem={null}
        onSave={onSave}
        onCancel={vi.fn()}
      />,
    );

    const ssh = screen.getByRole("checkbox", { name: /ssh/ });
    expect(ssh.hasAttribute("data-disabled")).toBe(true);

    /* Unavailable means genuinely inert, not merely styled to look
     * that way: a click must not register as a touch the save writes
     * out as an override. */
    await user.click(ssh);
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(onSave.mock.calls[0]?.[0]).not.toHaveProperty("ssh");
  });

  it("makes a capability editable once the catalogue reports it implemented", async () => {
    const user = userEvent.setup();
    const onSave = vi.fn();
    const implementedNow = catalogue().map((c) =>
      c.capability === "ssh" ? { ...c, implemented: true } : c,
    );
    render(
      <SandboxDialog
        project={project()}
        catalogue={implementedNow}
        problem={null}
        onSave={onSave}
        onCancel={vi.fn()}
      />,
    );

    const ssh = screen.getByRole("checkbox", { name: /ssh/ });
    expect(ssh.hasAttribute("data-disabled")).toBe(false);

    await user.click(ssh);
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(onSave).toHaveBeenCalledWith(
      expect.objectContaining({ ssh: false }),
    );
  });

  it("keeps a refusal visible inside the dialog", () => {
    render(
      <SandboxDialog
        project={project()}
        problem={{
          code: "sandbox_profile_invalid",
          message: "`srv/x` is not an absolute path",
          remediation: "give the path starting with /",
        }}
        onSave={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    const dialog = screen.getByRole("dialog");
    const alert = screen.getByRole("alert");

    expect(dialog.contains(alert)).toBe(true);
    expect(alert.textContent).toContain("sandbox_profile_invalid");
  });
});
