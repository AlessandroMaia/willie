import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Header } from "@/app/shell/header";
import { Sidebar, SidebarProvider } from "@/components/ui/sidebar";

/* Every call the header can make on the window goes through this one
 * mocked module; the asserted fns are hoisted so the mock factory
 * (evaluated before imports run) can close over them. `onResized` is
 * hoisted too — not asserted by most tests, but one test below
 * overrides its return value with a promise it controls, to make the
 * unmount-before-resolve race reproducible. */
const { minimize, toggleMaximize, close, onResized } = vi.hoisted(() => ({
  minimize: vi.fn(),
  toggleMaximize: vi.fn(),
  close: vi.fn(),
  onResized: vi.fn(async () => () => {}),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    minimize,
    toggleMaximize,
    close,
    isMaximized: vi.fn(async () => false),
    onResized,
  }),
}));

function renderHeader() {
  return render(
    <SidebarProvider>
      <Header />
      <Sidebar />
    </SidebarProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("the header", () => {
  it("the_window_controls_call_minimize_toggle_maximize_and_close", async () => {
    const user = userEvent.setup();
    renderHeader();
    await screen.findByRole("button", { name: "Minimize" });

    await user.click(screen.getByRole("button", { name: "Minimize" }));
    await user.click(screen.getByRole("button", { name: /Maximize|Restore/ }));
    await user.click(screen.getByRole("button", { name: "Close" }));

    expect(minimize).toHaveBeenCalledTimes(1);
    expect(toggleMaximize).toHaveBeenCalledTimes(1);
    expect(close).toHaveBeenCalledTimes(1);
  });

  it("the_centre_is_a_drag_region", async () => {
    renderHeader();

    const title = await screen.findByText("Willie");

    expect(title.getAttribute("data-tauri-drag-region")).not.toBeNull();
    expect(
      title.parentElement?.getAttribute("data-tauri-drag-region"),
    ).not.toBeNull();
  });

  /* The header, the sidebar and the status bar share one ground, so a
   * rule under the title bar would draw a line across it. The window
   * is 28px of chrome; every control in it has to fit inside that. */
  it("the_title_bar_carries_no_rule_and_its_controls_fit_its_height", async () => {
    renderHeader();

    const close = await screen.findByRole("button", { name: "Close" });
    const bar = close.closest("header");

    expect(bar?.className).toContain("h-(--header-height)");
    expect(bar?.className).not.toContain("border-b");

    for (const name of ["Minimize", "Close", "Toggle sidebar", "Open setup"]) {
      expect(screen.getByRole("button", { name }).className).toContain(
        "size-7",
      );
    }
  });

  it("the_sidebar_toggle_collapses_the_sidebar", async () => {
    const user = userEvent.setup();
    renderHeader();
    const sidebar = document.querySelector("[data-slot='sidebar'][data-state]");
    expect(sidebar?.getAttribute("data-state")).toBe("expanded");

    await user.click(screen.getByRole("button", { name: "Toggle sidebar" }));

    expect(sidebar?.getAttribute("data-state")).toBe("collapsed");
  });

  it("unmounting_before_onresized_resolves_still_unlistens", async () => {
    const stop = vi.fn();
    let resolveOnResized: (fn: () => void) => void = () => {};
    onResized.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveOnResized = resolve;
      }),
    );

    const { unmount } = renderHeader();
    unmount();
    resolveOnResized(stop);

    await vi.waitFor(() => expect(stop).toHaveBeenCalledTimes(1));
  });
});
