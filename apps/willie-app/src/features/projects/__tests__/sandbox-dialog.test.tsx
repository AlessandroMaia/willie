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
    default_enabled: true,
  },
  {
    capability: "agent_state",
    display_name: "agent.state",
    consequence: "anything the agent runs can use the harness's login",
    implemented: true,
    default_enabled: true,
  },
  {
    capability: "ssh",
    display_name: "ssh",
    consequence: "the agent can use the user's ssh keys and agent socket",
    implemented: false,
    default_enabled: false,
  },
];

/* A harness that leaves the credential off, which is what the `Harness`
 * trait's own default does: Claude Code is the only implementation that
 * turns `agent.state` on, so the dialog may not assume it. */
const credentialOffByDefault = (): CapabilityInfo[] =>
  catalogue().map((c) =>
    c.capability === "agent_state" ? { ...c, default_enabled: false } : c,
  );

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

    /* Off in the catalogue's defaults, so the one click that becomes
     * possible writes the override that turns it on. */
    await user.click(ssh);
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ ssh: true }));
  });

  it("seeds a row the profile says nothing about from the harness default", () => {
    render(
      <SandboxDialog
        project={project()}
        catalogue={credentialOffByDefault()}
        problem={null}
        onSave={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    const credential = screen.getByRole("checkbox", { name: /agent\.state/ });
    expect(credential.hasAttribute("data-checked")).toBe(false);
  });

  it("shows an override that turns a default-off capability on", () => {
    render(
      <SandboxDialog
        project={{ ...project(), sandbox: { agent_state: true } }}
        catalogue={credentialOffByDefault()}
        problem={null}
        onSave={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    const credential = screen.getByRole("checkbox", { name: /agent\.state/ });
    expect(credential.hasAttribute("data-checked")).toBe(true);
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

  /* Ten capabilities, each with the sentence that says what it costs,
   * are taller than a small window. A dialog that grows with them puts
   * Save past the bottom edge, where no amount of scrolling reaches it,
   * because the popup is centred rather than scrolled. So the rows
   * scroll inside a frame the dialog holds fixed, and the footer sits
   * outside that frame. */
  it("scrolls the capability rows and leaves the footer outside", () => {
    render(
      <SandboxDialog
        project={project()}
        catalogue={catalogue()}
        problem={null}
        onSave={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    const dialog = screen.getByRole("dialog");
    const rows = dialog.querySelector('[data-slot="scroll-area"]');
    const save = screen.getByRole("button", { name: "Save" });

    expect(rows).not.toBeNull();
    expect(rows?.textContent).toContain("agent.state");
    expect(rows?.contains(save)).toBe(false);
  });

  /* A refusal is the answer to pressing Save, so it belongs beside the
   * button and not in the part that scrolls, where it could be out of
   * sight at the moment it appears. */
  it("keeps a refusal out of the scrolling rows", () => {
    render(
      <SandboxDialog
        project={project()}
        catalogue={catalogue()}
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
    const rows = dialog.querySelector('[data-slot="scroll-area"]');
    const alert = screen.getByRole("alert");

    expect(rows?.contains(alert)).toBe(false);
  });
});
