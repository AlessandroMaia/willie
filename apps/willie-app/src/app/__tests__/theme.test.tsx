import { describe, expect, it } from "vitest";
import { followSystemTheme } from "@/app/theme";

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
