import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { SandboxDrawer } from "@/features/sandbox/sandbox-drawer";
import type { CapabilityInfo, Project, SandboxProfile } from "@/lib/proto";

/* Nothing overridden: every capability comes from the harness. */
const inherited: SandboxProfile = { extra_paths: [] };

/* The catalogue the Rust side owns, trimmed to what these tests read.
 * The drawer takes it as a prop, so no module is faked here. */
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
 * turns `agent.state` on, so the drawer may not assume it. */
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

describe("SandboxDrawer", () => {
  it("shows each capability with the sentence that says what it costs", () => {
    render(
      <SandboxDrawer
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
      <SandboxDrawer
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
      <SandboxDrawer
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
      <SandboxDrawer
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
      <SandboxDrawer
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
      <SandboxDrawer
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

  it("keeps a refusal visible inside the drawer", () => {
    render(
      <SandboxDrawer
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

    const drawer = screen.getByRole("dialog");
    const alert = screen.getByRole("alert");

    expect(drawer.contains(alert)).toBe(true);
    expect(alert.textContent).toContain("sandbox_profile_invalid");
  });

  /* Ten capabilities, each with the sentence that says what it costs,
   * are taller than a small window. The sheet is held to the viewport
   * rather than growing with its content, so the rows scroll inside a
   * frame it holds fixed and the footer sits outside that frame. */
  it("scrolls the capability rows and leaves the footer outside", () => {
    render(
      <SandboxDrawer
        project={project()}
        catalogue={catalogue()}
        problem={null}
        onSave={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    const drawer = screen.getByRole("dialog");
    const rows = drawer.querySelector('[data-slot="scroll-area"]');
    const save = screen.getByRole("button", { name: "Save" });

    expect(rows).not.toBeNull();
    expect(rows?.textContent).toContain("agent.state");
    expect(rows?.contains(save)).toBe(false);
  });

  /* A `sandbox_problem` is a load failure, not a rejected save: it comes
   * from the project itself, is present the instant the drawer opens,
   * and stays even though `problem` (the save-rejection prop) is null. */
  it("shows an alert when the project's sandbox table could not be read", () => {
    render(
      <SandboxDrawer
        project={{
          ...project(),
          sandbox_problem: {
            code: "sandbox_table_invalid",
            message: "the [sandbox] table could not be read",
            remediation: "open the Sandbox drawer and save to replace it",
          },
        }}
        catalogue={catalogue()}
        problem={null}
        onSave={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain(
      "the [sandbox] table could not be read",
    );
  });

  /* `local` must start from `{}`, not from `project.sandbox` (the
   * default profile the daemon substituted): every row falls back to
   * the harness's own default, proving the drawer never treats that
   * substitute as a saved override. */
  it("starts from the harness defaults, not the substituted profile, when the sandbox table could not be read", () => {
    render(
      <SandboxDrawer
        project={{
          ...project(),
          /* What the daemon loads a project with while the table can't
           * be read: the default profile, which happens to turn
           * `agent_state` off here — the opposite of what the
           * catalogue's own default says. If `local` started from this
           * instead of `{}`, the checkbox would come up unchecked. */
          sandbox: { agent_state: false },
          sandbox_problem: {
            code: "sandbox_table_invalid",
            message: "the [sandbox] table could not be read",
            remediation: "open the Sandbox drawer and save to replace it",
          },
        }}
        catalogue={catalogue()}
        problem={null}
        onSave={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    const credential = screen.getByRole("checkbox", { name: /agent\.state/ });
    expect(credential.hasAttribute("data-checked")).toBe(true);
  });

  /* A refusal is the answer to pressing Save, so it belongs beside the
   * button and not in the part that scrolls, where it could be out of
   * sight at the moment it appears. */
  it("keeps a refusal out of the scrolling rows", () => {
    render(
      <SandboxDrawer
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

    const drawer = screen.getByRole("dialog");
    const rows = drawer.querySelector('[data-slot="scroll-area"]');
    const alert = screen.getByRole("alert");

    expect(rows?.contains(alert)).toBe(false);
  });

  /* Sandbox capabilities are monotonic (docs/decisions/0007): a
   * repository's own configuration can only tighten this profile,
   * never open something it leaves off — the drawer says so up front. */
  it("shows the monotonic note about repository configuration", () => {
    render(
      <SandboxDrawer
        project={project()}
        catalogue={catalogue()}
        problem={null}
        onSave={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    expect(screen.getByText(/can only tighten/)).toBeDefined();
  });

  it("never offers an allow action from this drawer", () => {
    render(
      <SandboxDrawer
        project={project()}
        catalogue={catalogue()}
        problem={null}
        onSave={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    expect(screen.queryByRole("button", { name: /allow/i })).toBeNull();
  });
});
