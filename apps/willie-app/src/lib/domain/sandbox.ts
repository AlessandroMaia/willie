import type { Denied, SandboxState, Session } from "@/lib/proto";

/**
 * The applied sandbox mechanisms, joined into one line. Empty when
 * none were reported yet — a session still `creating`, or one recorded
 * before sandbox reporting existed.
 */
export function postureLine(sandbox: SandboxState): string {
  return sandbox.applied.join(" · ");
}

/** How many denials the sandbox recorded in total, across every kind. */
export function deniedCount(sandbox: SandboxState): number {
  return sandbox.denied.reduce((sum, d) => sum + d.count, 0);
}

export interface SystemPosture {
  applied: string[];
  unavailable: string[];
  degraded: string[];
}

/**
 * The system's mechanisms unioned across every one of its sessions —
 * live and finished alike — each in the order first seen. A session
 * with no `sandbox` field yet (still `creating`, or recorded before
 * sandbox reporting existed) contributes nothing rather than counting
 * as a failure.
 */
export function posture(sessions: Session[]): SystemPosture {
  const applied = new Set<string>();
  const unavailable = new Set<string>();
  const degraded = new Set<string>();

  for (const session of sessions) {
    const sandbox = session.sandbox;
    if (!sandbox) continue;
    for (const m of sandbox.applied) applied.add(m);
    for (const m of sandbox.unavailable) unavailable.add(m);
    for (const m of sandbox.degraded) degraded.add(m);
  }

  return {
    applied: [...applied],
    unavailable: [...unavailable],
    degraded: [...degraded],
  };
}

/** One denial flattened out of a session's sandbox report, carrying
 * the session it happened in so the system-wide history can still say
 * whose session it was. */
export interface DenialRow {
  sessionId: string;
  class: Denied["class"];
  name: string;
  count: number;
  lastAt: string;
}

/** Epoch seconds as a number, `-Infinity` (the oldest possible moment)
 * when the string does not parse — a malformed timestamp sorts to the
 * end of the history rather than crashing or claiming to be the
 * newest. */
function epochOf(value: string): number {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : Number.NEGATIVE_INFINITY;
}

/**
 * Every session's denials flattened into one chronological list across
 * the whole system, newest `last_at` first — the history a single
 * session's own report cannot show on its own.
 */
export function denialRows(sessions: Session[]): DenialRow[] {
  const rows: DenialRow[] = [];

  for (const session of sessions) {
    const sandbox = session.sandbox;
    if (!sandbox) continue;
    for (const denial of sandbox.denied) {
      rows.push({
        sessionId: session.id,
        class: denial.class,
        name: denial.name,
        count: denial.count,
        lastAt: denial.last_at,
      });
    }
  }

  return rows.sort((a, b) => epochOf(b.lastAt) - epochOf(a.lastAt));
}

export interface DenialCounts {
  syscalls: number;
  terminal: number;
  sessions: number;
}

/** The three headline counts: syscall and terminal denials summed
 * separately, plus how many distinct sessions have at least one —
 * never how many rows, since one session can carry several. */
export function counts(rows: DenialRow[]): DenialCounts {
  let syscalls = 0;
  let terminal = 0;
  const sessionsWithDenials = new Set<string>();

  for (const row of rows) {
    if (row.class === "syscall") syscalls += row.count;
    else terminal += row.count;
    sessionsWithDenials.add(row.sessionId);
  }

  return { syscalls, terminal, sessions: sessionsWithDenials.size };
}

const SYSCALL_EXPLANATIONS: Record<string, string> = {
  prctl: "the session tried to install its own syscall filter",
  ioctl: "injection into the controlling terminal",
};

const SYSCALL_FALLBACK = "a syscall this sandbox does not allow";

const TERMINAL_EXPLANATION =
  "a sequence that acts on the host, filtered from the terminal output";

/**
 * The one-line reason a denial happened, keyed on the wire's
 * `class`/`name` pair. Every `terminal` denial explains the same way —
 * the class itself is the reason, so an unrecognised terminal name
 * still reads the same. An unrecognised `syscall` name falls back to a
 * class-level sentence instead: the daemon can add denial names
 * without a frontend change, so this never throws and never returns
 * undefined.
 */
export function explain(cls: Denied["class"], name: string): string {
  if (cls === "terminal") return TERMINAL_EXPLANATION;
  return SYSCALL_EXPLANATIONS[name] ?? SYSCALL_FALLBACK;
}
