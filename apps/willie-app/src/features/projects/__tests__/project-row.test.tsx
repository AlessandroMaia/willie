import { render, screen } from "@testing-library/react";
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

function renderRow(project: Project) {
  render(
    <ProjectRow
      project={project}
      job={undefined}
      isEditing={false}
      editingName=""
      isBusy={false}
      jobRunning={false}
      path="\\\\wsl.localhost\\willie\\home\\willie\\projects\\willie"
      rowProblem={null}
      openNotice={null}
      live={0}
      canResume={true}
      onEditingNameChange={vi.fn()}
      onStartRename={vi.fn()}
      onSaveRename={vi.fn()}
      onCancelRename={vi.fn()}
      onRetry={vi.fn()}
      onCopyPath={vi.fn()}
      onOpenInExplorer={vi.fn()}
      onOpenRelocateDialog={vi.fn()}
      onOpenSandboxDialog={vi.fn()}
      onOpenSession={vi.fn()}
      onResumeSession={vi.fn()}
      onSyncToWindows={vi.fn()}
      onUpdateFromWindows={vi.fn()}
      onCancelJob={vi.fn()}
      onOpenRemoveDialog={vi.fn()}
    />,
  );
}

describe("ProjectRow", () => {
  it("holds Open session and Resume while the sandbox settings could not be read, but not the sync buttons", () => {
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
          name: "Open session",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    expect(
      (screen.getByRole("button", { name: "Resume" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
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

  it("leaves Open session and Resume enabled for a project without a sandbox problem", () => {
    renderRow(project());

    expect(
      (
        screen.getByRole("button", {
          name: "Open session",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(false);
    expect(
      (screen.getByRole("button", { name: "Resume" }) as HTMLButtonElement)
        .disabled,
    ).toBe(false);
  });
});
