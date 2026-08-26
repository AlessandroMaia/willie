import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { formatVersion } from "./lib/version";

/**
 * Application shell. Real screens (dashboard, projects, sessions, plugins)
 * arrive with their slices; this only proves the webview ↔ Rust bridge.
 */
export function App() {
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    invoke<string>("app_version")
      .then(setVersion)
      .catch(() => setVersion(null));
  }, []);

  return (
    <main className="shell">
      <h1>Willie</h1>
      <p className="muted">{formatVersion(version)}</p>
    </main>
  );
}
