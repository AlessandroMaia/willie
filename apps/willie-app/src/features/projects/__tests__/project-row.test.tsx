import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ProjectRow } from "@/features/projects/project-row";
import type { Project } from "@/lib/proto";

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
  sandbox: {},
});

function renderRow(
  project: Project,
  overrides: Partial<{
    isBusy: boolean;
    jobRunning: boolean;
    live: number;
  }> = {},
) {
  render(
    <ProjectRow
      project={project}
      job={undefined}
      isEditing={false}
      editingName=""
      isBusy={overrides.isBusy ?? false}
      jobRunning={overrides.jobRunning ?? false}
      path="\\\\wsl.localhost\\willie\\home\\willie\\projects\\willie"
      rowProblem={null}
      live={overrides.live ?? 0}
      onEditingNameChange={vi.fn()}
      onStartRename={vi.fn()}
      onSaveRename={vi.fn()}
      onCancelRename={vi.fn()}
      onRetry={vi.fn()}
      onCopyPath={vi.fn()}
      onOpenInExplorer={vi.fn()}
      onOpenRelocateDialog={vi.fn()}
      onSyncToWindows={vi.fn()}
      onUpdateFromWindows={vi.fn()}
      onCancelJob={vi.fn()}
      onOpenRemoveDialog={vi.fn()}
    />,
  );
}

describe("ProjectRow", () => {
  it("offers no session, sandbox or editor action — those belong to the system's own screens", async () => {
    const user = userEvent.setup();
    renderRow(project());

    expect(screen.queryByRole("button", { name: "Open session" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Resume" })).toBeNull();

    await user.click(screen.getByRole("button", { name: /More actions/ }));
    await screen.findByRole("menuitem", { name: "Rename" });

    expect(screen.queryByRole("menuitem", { name: "Sandbox…" })).toBeNull();
    expect(
      screen.queryByRole("menuitem", { name: "Open in VS Code" }),
    ).toBeNull();
  });

  it("never blocks Send to Windows or Update from Windows on a sandbox problem", () => {
    renderRow({
      ...project(),
      sandbox_problem: {
        code: "sandbox_table_invalid",
        message: "the [sandbox] table could not be read",
        remediation: "open the Sandbox dialog and save to replace it",
      },
    });

    expect(
      (
        screen.getByRole("button", {
          name: "Send to Windows",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(false);
    expect(
      (
        screen.getByRole("button", {
          name: "Update from Windows",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(false);
  });

  it("offers Rename, Copy workspace path, Open in Explorer and Remove", async () => {
    const user = userEvent.setup();
    renderRow(project());

    await user.click(screen.getByRole("button", { name: /More actions/ }));

    expect(
      await screen.findByRole("menuitem", { name: "Rename" }),
    ).toBeDefined();
    expect(
      screen.getByRole("menuitem", { name: "Copy workspace path" }),
    ).toBeDefined();
    expect(
      screen.getByRole("menuitem", { name: "Open in Explorer" }),
    ).toBeDefined();
    expect(screen.getByRole("menuitem", { name: "Remove…" })).toBeDefined();
  });

  it("offers Relocate source only while the source is missing", async () => {
    const user = userEvent.setup();
    renderRow({ ...project(), source_present: false });

    await user.click(screen.getByRole("button", { name: /More actions/ }));

    expect(
      await screen.findByRole("menuitem", { name: "Relocate source…" }),
    ).toBeDefined();
    expect(screen.getByText("source missing")).toBeDefined();
  });

  it("shows a live badge only when a session is live", () => {
    renderRow(project(), { live: 2 });

    expect(screen.getByText("2 live")).toBeDefined();
  });

  it("disables the sync buttons while busy or a job is running", () => {
    renderRow(project(), { isBusy: true, jobRunning: true });

    expect(
      (
        screen.getByRole("button", {
          name: "Send to Windows",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
  });
});
