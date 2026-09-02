import { HotkeysProvider } from "@tanstack/react-hotkeys";
import { RouterProvider } from "@tanstack/react-router";
import { useEffect } from "react";
import { type AppRouter, createAppRouter } from "@/app/router";
import { followSystemTheme } from "@/app/theme";
import { Toaster } from "@/components/ui/toast";
import { TooltipProvider } from "@/components/ui/tooltip";

const defaultRouter = createAppRouter();

interface AppProps {
  /** Tests pass a router over a memory history. */
  router?: AppRouter;
}

export function App({ router = defaultRouter }: AppProps) {
  useEffect(() => followSystemTheme(), []);

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
