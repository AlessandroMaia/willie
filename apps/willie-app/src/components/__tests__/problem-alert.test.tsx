import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ProblemAlert } from "@/components/problem-alert";

const problem = {
  code: "project_busy",
  message: "A job is already running",
  remediation: "Wait for it to finish",
};

describe("ProblemAlert", () => {
  it("renders the code, the message and the remediation as an alert", () => {
    render(<ProblemAlert problem={problem} />);

    const alert = screen.getByRole("alert");

    expect(alert.textContent).toContain("project_busy");
    expect(alert.textContent).toContain("A job is already running");
    expect(alert.textContent).toContain("Wait for it to finish");
  });

  it("renders a notice as a status, without the code", () => {
    render(<ProblemAlert problem={problem} tone="notice" />);

    const status = screen.getByRole("status");

    expect(status.textContent).toContain("A job is already running");
    expect(status.textContent).not.toContain("project_busy");
  });
});
