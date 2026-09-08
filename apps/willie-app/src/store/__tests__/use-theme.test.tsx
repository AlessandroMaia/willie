import { renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { applyTheme, followSystemTheme, useThemeMode } from "@/store/use-theme";

function fakeMedia(initial: boolean) {
  const listeners = new Set<() => void>();
  let matches = initial;
  return {
    get matches() {
      return matches;
    },
    addEventListener(_type: "change", cb: () => void) {
      listeners.add(cb);
    },
    removeEventListener(_type: "change", cb: () => void) {
      listeners.delete(cb);
    },
    flip() {
      matches = !matches;
      for (const cb of listeners) cb();
    },
    listenerCount: () => listeners.size,
  };
}

describe("followSystemTheme", () => {
  it("applies .dark when the system prefers dark and removes it when it stops", () => {
    const root = document.createElement("html");
    const media = fakeMedia(true);

    const stop = followSystemTheme(root, media);
    expect(root.classList.contains("dark")).toBe(true);

    media.flip();
    expect(root.classList.contains("dark")).toBe(false);

    stop();
    expect(media.listenerCount()).toBe(0);
  });

  it("treats a missing media query as light", () => {
    const root = document.createElement("html");
    root.classList.add("dark");

    followSystemTheme(root, null);

    expect(root.classList.contains("dark")).toBe(false);
  });
});

describe("applyTheme", () => {
  it('pins dark regardless of the system preference for mode "dark"', () => {
    const root = document.createElement("html");
    const media = fakeMedia(false);

    applyTheme("dark", root, media);

    expect(root.classList.contains("dark")).toBe(true);
    expect(media.listenerCount()).toBe(0);
  });

  it('pins light regardless of the system preference for mode "light"', () => {
    const root = document.createElement("html");
    const media = fakeMedia(true);

    applyTheme("light", root, media);

    expect(root.classList.contains("dark")).toBe(false);
    expect(media.listenerCount()).toBe(0);
  });

  it('follows the media query live for mode "system"', () => {
    const root = document.createElement("html");
    const media = fakeMedia(false);

    applyTheme("system", root, media);
    expect(root.classList.contains("dark")).toBe(false);

    media.flip();
    expect(root.classList.contains("dark")).toBe(true);
  });
});

describe("useThemeMode", () => {
  it("starts on system and updates every subscriber on setMode", () => {
    const a = renderHook(() => useThemeMode());
    const b = renderHook(() => useThemeMode());
    expect(a.result.current.mode).toBe("system");

    a.result.current.setMode("dark");
    a.rerender();
    b.rerender();

    expect(a.result.current.mode).toBe("dark");
    expect(b.result.current.mode).toBe("dark");

    a.result.current.setMode("system");
    a.rerender();
  });
});
