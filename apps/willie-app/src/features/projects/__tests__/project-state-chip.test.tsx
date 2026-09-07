import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ProjectStateChip } from "@/features/projects/project-state-chip";
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

describe("ProjectStateChip", () => {
  it("shows a warning chip with the reason when the sandbox table could not be read", () => {
    render(
      <ProjectStateChip
        project={{
          ...project(),
          sandbox_problem: {
            code: "sandbox_table_invalid",
            message: "the [sandbox] table could not be read",
            remediation: "open the Sandbox dialog and save to replace it",
          },
        }}
        job={undefined}
        onRetry={vi.fn()}
      />,
    );

    expect(screen.getByText("sandbox_table_invalid")).toBeDefined();
    expect(
      screen.getByText("the [sandbox] table could not be read"),
    ).toBeDefined();
  });

  it("shows ready for a project without a sandbox problem", () => {
    render(
      <ProjectStateChip
        project={project()}
        job={undefined}
        onRetry={vi.fn()}
      />,
    );

    expect(screen.getByText("ready")).toBeDefined();
    expect(screen.queryByText("sandbox_table_invalid")).toBeNull();
  });
});
