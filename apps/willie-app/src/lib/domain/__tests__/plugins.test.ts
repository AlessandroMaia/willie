import { describe, expect, it } from "vitest";
import { isPluginDisabled } from "@/lib/domain/plugins";

describe("isPluginDisabled", () => {
  it("is true for the literal plugin_disabled code", () => {
    expect(
      isPluginDisabled({
        code: "plugin_disabled",
        message: "the profile plugin is not enabled for this project",
        remediation: "",
      }),
    ).toBe(true);
  });

  it("is true for a flattened daemon_error whose message names plugin_disabled", () => {
    expect(
      isPluginDisabled({
        code: "daemon_error",
        message: "rpc failed: plugin_disabled: profile is not enabled",
        remediation: "",
      }),
    ).toBe(true);
  });

  it("is false for a daemon_error unrelated to plugin enablement", () => {
    expect(
      isPluginDisabled({
        code: "daemon_error",
        message: "the daemon is not reachable",
        remediation: "",
      }),
    ).toBe(false);
  });

  it("is false for an unrelated problem code", () => {
    expect(
      isPluginDisabled({
        code: "project_busy",
        message: "a job is already running",
        remediation: "",
      }),
    ).toBe(false);
  });

  it("is false for a value that carries no code or message", () => {
    expect(isPluginDisabled(new Error("boom"))).toBe(false);
  });
});
