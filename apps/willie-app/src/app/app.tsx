import { HotkeysProvider } from "@tanstack/react-hotkeys";
import { RouterProvider } from "@tanstack/react-router";
import { useEffect } from "react";
import { type AppRouter, createAppRouter } from "@/app/router";
import { Toaster } from "@/components/ui/toast";
import { TooltipProvider } from "@/components/ui/tooltip";
import { applyTheme, useThemeMode } from "@/store/use-theme";

const defaultRouter = createAppRouter();

interface AppProps {
  /** Tests pass a router over a memory history. */
  router?: AppRouter;
}

export function App({ router = defaultRouter }: AppProps) {
  const { mode } = useThemeMode();
  useEffect(() => applyTheme(mode), [mode]);

  return (
    <HotkeysProvider>
      <TooltipProvider>
        <Toaster>
          <RouterProvider router={router} />
        </Toaster>
      </TooltipProvider>
    </HotkeysProvider>
  );
}
