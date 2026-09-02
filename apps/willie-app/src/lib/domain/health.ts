import type { EngineStatus } from "@/lib/ipc";

/** A fact about one part of the engine, or about all of it: working,
 * usable but incomplete, or broken. How it looks is decided by the
 * component that renders it, never here. */
export type Health = "ok" | "degraded" | "failed";

export type Part = "wsl" | "distro" | "daemon" | "doctor";

/** The order a summary reads the parts in: the first that is not ok
 * is the one worth naming, and WSL comes before everything that runs
 * inside it. */
export const PARTS: readonly Part[] = ["wsl", "distro", "daemon", "doctor"];

export function healthFor(part: Part, s: EngineStatus): Health {
  switch (part) {
    case "wsl":
      return s.wsl.installed && s.wsl.meets_minimum ? "ok" : "failed";
    case "distro":
      if (s.distro_error) return "failed";
      return s.distro?.registered ? "ok" : "degraded";
    case "daemon":
      if (s.daemon.state === "failed") return "failed";
      return s.daemon.state === "running" ? "ok" : "degraded";
    case "doctor":
      if (!s.doctor) return "degraded";
      return s.doctor.checks.some((c) => c.required && c.status === "fail")
        ? "failed"
        : "ok";
  }
}

const RANK: Record<Health, number> = { ok: 0, degraded: 1, failed: 2 };

export function overallHealth(s: EngineStatus): Health {
  return PARTS.map((p) => healthFor(p, s)).reduce(
    (worst, h) => (RANK[h] > RANK[worst] ? h : worst),
    "ok",
  );
}
