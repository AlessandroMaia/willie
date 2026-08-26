import type { EngineStatus } from "./engine";

export type Light = "red" | "yellow" | "green";
export type Part = "wsl" | "distro" | "daemon" | "doctor";

export function lightFor(part: Part, s: EngineStatus): Light {
  switch (part) {
    case "wsl":
      return s.wsl.installed && s.wsl.meets_minimum ? "green" : "red";
    case "distro":
      if (s.distro_error) return "red";
      return s.distro?.registered ? "green" : "yellow";
    case "daemon":
      if (s.daemon.state === "failed") return "red";
      return s.daemon.state === "running" ? "green" : "yellow";
    case "doctor":
      if (!s.doctor) return "yellow";
      return s.doctor.checks.some((c) => c.required && c.status === "fail")
        ? "red"
        : "green";
  }
}

const RANK: Record<Light, number> = { green: 0, yellow: 1, red: 2 };

export function overallHealth(s: EngineStatus): Light {
  const parts: Part[] = ["wsl", "distro", "daemon", "doctor"];
  return parts
    .map((p) => lightFor(p, s))
    .reduce((worst, l) => (RANK[l] > RANK[worst] ? l : worst), "green");
}
