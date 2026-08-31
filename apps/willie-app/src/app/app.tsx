import { useState } from "react";
import { ROUTES } from "@/app/routes";

export function App() {
  const [active, setActive] = useState(ROUTES[0]?.id ?? "");
  const route = ROUTES.find((r) => r.id === active) ?? ROUTES[0];
  const Screen = route?.screen;
  return (
    <div className="app">
      <nav className="tabs">
        {ROUTES.map((r) => (
          <button
            key={r.id}
            type="button"
            data-active={r.id === active}
            onClick={() => setActive(r.id)}
          >
            {r.label}
          </button>
        ))}
      </nav>
      {Screen && <Screen />}
    </div>
  );
}
