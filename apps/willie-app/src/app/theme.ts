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
 * returns the cleanup. No media query means light. A user override
 * from Settings is one more argument here, later, and touches no
 * component: everything below reads the class, never the preference.
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
