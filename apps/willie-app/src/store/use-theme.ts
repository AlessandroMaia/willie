import { useSyncExternalStore } from "react";

/** The part of MediaQueryList the theme needs; a test passes a fake. */
export interface ThemeMedia {
  readonly matches: boolean;
  addEventListener(type: "change", cb: () => void): void;
  removeEventListener(type: "change", cb: () => void): void;
}

const QUERY = "(prefers-color-scheme: dark)";

function systemMedia(): ThemeMedia | null {
  return typeof window !== "undefined" &&
    typeof window.matchMedia === "function"
    ? window.matchMedia(QUERY)
    : null;
}

/**
 * Keeps `.dark` on the root in step with the system preference and
 * returns the cleanup. No media query means light. Used as-is when
 * `useThemeMode()` reports `"system"`; `applyTheme` below is what
 * chooses between this and a pinned light/dark.
 */
export function followSystemTheme(
  root: HTMLElement = document.documentElement,
  media: ThemeMedia | null = systemMedia(),
): () => void {
  const apply = () => root.classList.toggle("dark", media?.matches ?? false);

  apply();
  if (!media) return () => {};

  media.addEventListener("change", apply);
  return () => media.removeEventListener("change", apply);
}

export type ThemeMode = "system" | "light" | "dark";

interface ThemeModeState {
  mode: ThemeMode;
}

type Listener = () => void;

let state: ThemeModeState = { mode: "system" };
const listeners = new Set<Listener>();

function setState(next: ThemeModeState): void {
  state = next;
  for (const listener of listeners) listener();
}

function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getState(): ThemeModeState {
  return state;
}

function setMode(mode: ThemeMode): void {
  setState({ mode });
}

export interface ThemeModeHandle {
  mode: ThemeMode;
  setMode: (mode: ThemeMode) => void;
}

/**
 * The user's theme override, a module-level singleton so the Settings
 * screen and the app root agree on it without a shared ancestor. Not
 * persisted: it resets to "system" on every launch, the same as
 * before this existed.
 */
export function useThemeMode(): ThemeModeHandle {
  const snapshot = useSyncExternalStore(subscribe, getState);
  return { mode: snapshot.mode, setMode };
}

/**
 * Applies one `ThemeMode` to the root and returns the cleanup:
 * "system" follows the media query live (`followSystemTheme`),
 * "light"/"dark" pin the class regardless of it. Mounted once at the
 * app root, re-run whenever `useThemeMode()`'s mode changes.
 */
export function applyTheme(
  mode: ThemeMode,
  root: HTMLElement = document.documentElement,
  media: ThemeMedia | null = systemMedia(),
): () => void {
  if (mode === "system") return followSystemTheme(root, media);

  root.classList.toggle("dark", mode === "dark");
  return () => {};
}
