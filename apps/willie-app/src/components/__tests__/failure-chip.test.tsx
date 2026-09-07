import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { FailureChip } from "@/components/failure-chip";

describe("FailureChip", () => {
  it("renders the destructive surface by default", () => {
    render(
      <FailureChip
        code="project_busy"
        message="A job is already running"
        remediation="Wait for it"
      />,
    );

    const code = screen.getByText("project_busy");
    expect(code.parentElement?.className).toContain("bg-destructive/10");
    expect(code.parentElement?.className).toContain("text-destructive");
  });

  it("renders the warning surface when given the warning tone", () => {
    render(
      <FailureChip
        tone="warning"
        code="sandbox_table_invalid"
        message="the [sandbox] table could not be read"
        remediation="open the Sandbox dialog and save to replace it"
      />,
    );

    const code = screen.getByText("sandbox_table_invalid");
    expect(code.parentElement?.className).toContain("bg-warning/15");
    expect(code.parentElement?.className).toContain("text-warning");
    expect(code.parentElement?.className).not.toContain("bg-destructive/10");
  });
});
