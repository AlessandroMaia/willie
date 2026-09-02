import { describe, expect, it } from "vitest";
import { healthFor, overallHealth } from "@/lib/domain/health";
import type { DistroStatus, EngineStatus } from "@/lib/ipc";

const distro: DistroStatus = {
  registered: true,
  running: false,
  install_dir: "C:\\x",
};

const base: EngineStatus = {
  engine_version: "0.1.0",
  wsl: {
    installed: true,
    version: "2.6.1.0",
    meets_minimum: true,
    minimum: "2.4.4",
  },
  distro,
  distro_error: null,
  daemon: {
    state: "running",
    willie_version: "0.1.0",
    image_version: "0.1.0+abc",
  },
  doctor: {
    checks: [
      {
        name: "a",
        status: "ok",
        detail: "",
        remediation: null,
        required: true,
      },
    ],
  },
  image_available: true,
};

describe("health lights", () => {
  it("is ok when wsl, distro, daemon and doctor are all fine", () => {
    expect(overallHealth(base)).toBe("ok");
  });

  it("has failed when WSL is missing or too old", () => {
    expect(
      overallHealth({ ...base, wsl: { ...base.wsl, meets_minimum: false } }),
    ).toBe("failed");
    expect(
      healthFor("wsl", { ...base, wsl: { ...base.wsl, installed: false } }),
    ).toBe("failed");
  });

  it("is degraded when the distro is not registered yet", () => {
    expect(
      overallHealth({
        ...base,
        distro: { ...distro, registered: false },
        daemon: { state: "stopped" },
      }),
    ).toBe("degraded");
  });

  it("has failed when a required doctor check fails or the daemon failed", () => {
    const failing = {
      checks: [
        {
          name: "a",
          status: "fail" as const,
          detail: "",
          remediation: "x",
          required: true,
        },
      ],
    };
    expect(overallHealth({ ...base, doctor: failing })).toBe("failed");
    expect(
      healthFor("daemon", {
        ...base,
        daemon: { state: "failed", code: "daemon_exited", message: "boom" },
      }),
    ).toBe("failed");
  });

  it("treats a stopped daemon as degraded, not failed", () => {
    expect(healthFor("daemon", { ...base, daemon: { state: "stopped" } })).toBe(
      "degraded",
    );
  });
});
