import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { SettingsScreen } from "@/features/settings/settings-screen";

describe("SettingsScreen", () => {
  it("marks the chosen theme mode as pressed and no other", async () => {
    const user = userEvent.setup();
    render(<SettingsScreen />);
    const system = screen.getByRole("button", { name: "System" });
    const dark = screen.getByRole("button", { name: "Dark" });
    expect(system.getAttribute("aria-pressed")).toBe("true");

    await user.click(dark);

    expect(dark.getAttribute("aria-pressed")).toBe("true");
    expect(system.getAttribute("aria-pressed")).toBe("false");

    /* Restored so a later test added to this file never inherits a
     * "dark" mode this one left behind — `useThemeMode` is a
     * module-level singleton for the file's whole run. */
    await user.click(system);
  });
});
