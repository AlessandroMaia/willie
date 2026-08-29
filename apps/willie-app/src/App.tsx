import { useState } from "react";
import { Dashboard } from "./screens/Dashboard";
import { Projects } from "./screens/Projects";
import { Sessions } from "./screens/Sessions";

type Tab = "dashboard" | "projects" | "sessions";

export function App() {
  const [tab, setTab] = useState<Tab>("dashboard");
  return (
    <div className="app">
      <nav className="tabs">
        <button
          type="button"
          data-active={tab === "dashboard"}
          onClick={() => setTab("dashboard")}
        >
          Dashboard
        </button>
        <button
          type="button"
          data-active={tab === "projects"}
          onClick={() => setTab("projects")}
        >
          Projects
        </button>
        <button
          type="button"
          data-active={tab === "sessions"}
          onClick={() => setTab("sessions")}
        >
          Sessions
        </button>
      </nav>
      {tab === "dashboard" ? (
        <Dashboard />
      ) : tab === "projects" ? (
        <Projects />
      ) : (
        <Sessions />
      )}
    </div>
  );
}
