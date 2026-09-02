import { type Hotkey, useHotkey } from "@tanstack/react-hotkeys";
import { useNavigate } from "@tanstack/react-router";
import { ROUTES, type ScreenPath } from "@/app/routes";
import { useSidebar } from "@/components/ui/sidebar";

interface ScreenHotkeyProps {
  shortcut: string;
  path: ScreenPath;
}

/* One hook call per available screen, each in its own component, so
 * the registry can grow without a hook inside a loop. The registry
 * keeps `shortcut` a plain string (it is data, not a dependency on the
 * shortcuts library's type); the cast here is the one place that
 * trusts it matches the library's stricter `Hotkey` literal union. */
function ScreenHotkey({ shortcut, path }: ScreenHotkeyProps) {
  const navigate = useNavigate();

  useHotkey(shortcut as Hotkey, () => void navigate({ to: path }), {
    preventDefault: true,
  });

  return null;
}

/* The generated sidebar also listens for `Ctrl+B` on `window`. The
 * shortcuts library handles the chord on `document` first and stops
 * propagation, so the sidebar toggles exactly once — the shell test
 * pins that. */
function SidebarHotkey() {
  const { toggleSidebar } = useSidebar();

  useHotkey("Mod+B", () => toggleSidebar(), { preventDefault: true });

  return null;
}

/** The shell's shortcuts: Mod+1…N for the available screens, in
 * registry order, and Mod+B for the sidebar. Rendered inside the
 * SidebarProvider and the RouterProvider, which both hooks need. */
export function ShellHotkeys() {
  return (
    <>
      {ROUTES.filter((r) => r.available).map((r) => (
        <ScreenHotkey key={r.id} shortcut={r.shortcut} path={r.path} />
      ))}
      <SidebarHotkey />
    </>
  );
}
