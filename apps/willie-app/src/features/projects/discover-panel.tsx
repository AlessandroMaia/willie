import type { Candidate } from "@/lib/proto";

interface DiscoverPanelProps {
  candidates: Candidate[] | null;
  selected: Set<string>;
  busy: boolean;
  onToggle: (path: string) => void;
  onAddSelected: () => void;
}

export function DiscoverPanel({
  candidates,
  selected,
  busy,
  onToggle,
  onAddSelected,
}: DiscoverPanelProps) {
  if (candidates === null) return null;
  return (
    <section className="discover">
      <h2>Discovered</h2>
      {candidates.length === 0 ? (
        <p className="muted">No repositories found under the roots.</p>
      ) : (
        <>
          <ul>
            {candidates.map((c) => (
              <li key={c.path} className="candidate">
                <label>
                  <input
                    type="checkbox"
                    checked={selected.has(c.path)}
                    onChange={() => onToggle(c.path)}
                  />
                  {c.name}
                  <span className="muted"> — {c.path}</span>
                </label>
              </li>
            ))}
          </ul>
          <button
            type="button"
            disabled={selected.size === 0 || busy}
            onClick={onAddSelected}
          >
            Add selected
          </button>
        </>
      )}
    </section>
  );
}
