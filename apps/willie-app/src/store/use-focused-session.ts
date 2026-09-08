import { useSyncExternalStore } from "react";
import type { Session } from "@/lib/proto";
import { useEngineStatus } from "@/store/use-engine-status";
import { useSnapshot } from "@/store/use-snapshot";

type Listener = () => void;

let focusedId: string | null = null;
const listeners = new Set<Listener>();

function setState(next: string | null): void {
  focusedId = next;
  for (const listener of listeners) listener();
}

function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getState(): string | null {
  return focusedId;
}

/** Back to "no tab focused". Only a test harness calls this; the
 * Session screen clears the focus itself when it unmounts. */
export function resetForTests(): void {
  setState(null);
}

export interface FocusedSessionHandle {
  session: Session | null;
  sessionId: string | null;
  setFocused: (id: string | null) => void;
}

/**
 * The one session the shell treats as focused: only the id is
 * module-level state, the same style as `use-current-system`; the
 * session object itself always comes fresh off the daemon snapshot; so
 * a session that changes underneath (sandbox denials, state) is never
 * stale here. Task 12 calls `setFocused` from the active session tab;
 * nothing does yet, so `session` stays null until then.
 */
export function useFocusedSession(): FocusedSessionHandle {
  const sessionId = useSyncExternalStore(subscribe, getState);
  const { status } = useEngineStatus();
  const daemonRunning = status?.daemon.state === "running";
  const { snapshot } = useSnapshot(daemonRunning);

  const session =
    sessionId !== null
      ? (snapshot?.sessions.find((s) => s.id === sessionId) ?? null)
      : null;

  return { session, sessionId, setFocused: setState };
}
