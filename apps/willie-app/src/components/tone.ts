import type { Health } from "@/lib/domain/health";

/** The design system's vocabulary for the state of a thing. Every
 * coloured state in the app goes through one of these five; the
 * classes below are the only place a tone meets a colour token. */
export type Tone = "ok" | "warning" | "error" | "pending" | "muted";

export const TONE_TEXT: Record<Tone, string> = {
  ok: "text-success",
  warning: "text-warning",
  error: "text-destructive",
  pending: "text-info",
  muted: "text-muted-foreground",
};

export const TONE_SURFACE: Record<Tone, string> = {
  ok: "bg-success/15 text-success",
  warning: "bg-warning/15 text-warning",
  error: "bg-destructive/10 text-destructive",
  pending: "bg-info/15 text-info",
  muted: "bg-muted text-muted-foreground",
};

export const TONE_DOT: Record<Tone, string> = {
  ok: "bg-success",
  warning: "bg-warning",
  error: "bg-destructive",
  pending: "bg-info",
  muted: "bg-muted-foreground",
};

export function toneForHealth(health: Health): Tone {
  switch (health) {
    case "ok":
      return "ok";
    case "degraded":
      return "warning";
    case "failed":
      return "error";
  }
}
