import { describe, expect, it } from "vitest";
import { isShellShortcut } from "@/features/sessions/session-terminal";

function keydown(
  key: string,
  modifiers: { ctrlKey?: boolean; altKey?: boolean; shiftKey?: boolean } = {},
): KeyboardEvent {
  return new KeyboardEvent("keydown", {
    key,
    ctrlKey: modifiers.ctrlKey ?? false,
    altKey: modifiers.altKey ?? false,
    shiftKey: modifiers.shiftKey ?? false,
  });
}

describe("isShellShortcut", () => {
  it("lets Ctrl and a digit reach the shell", () => {
    expect(isShellShortcut(keydown("3", { ctrlKey: true }))).toBe(true);
  });

  it("keeps Ctrl+B with the session", () => {
    expect(isShellShortcut(keydown("b", { ctrlKey: true }))).toBe(false);
  });

  it("keeps a digit chord that also holds Shift with the session", () => {
    expect(
      isShellShortcut(keydown("3", { ctrlKey: true, shiftKey: true })),
    ).toBe(false);
  });

  it("keeps a digit chord that also holds Alt with the session", () => {
    expect(isShellShortcut(keydown("3", { ctrlKey: true, altKey: true }))).toBe(
      false,
    );
  });

  it("keeps a bare digit with the session", () => {
    expect(isShellShortcut(keydown("3"))).toBe(false);
  });

  it("keeps Ctrl+0 with the session, since no screen answers to it", () => {
    expect(isShellShortcut(keydown("0", { ctrlKey: true }))).toBe(false);
  });
});
