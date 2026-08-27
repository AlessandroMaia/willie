import { useState } from "react";
import { Dashboard } from "./screens/Dashboard";
import { Projects } from "./screens/Projects";

type Tab = "dashboard" | "projects";

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
      </nav>
      {tab === "dashboard" ? <Dashboard /> : <Projects />}
    </div>
  );
}
