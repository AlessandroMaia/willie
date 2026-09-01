interface RootsPanelProps {
  roots: string[];
  newRoot: string;
  onNewRootChange: (value: string) => void;
  busy: boolean;
  onAdd: () => void;
  onRemove: (root: string) => void;
  discovering: boolean;
  onDiscover: () => void;
}

export function RootsPanel({
  roots,
  newRoot,
  onNewRootChange,
  busy,
  onAdd,
  onRemove,
  discovering,
  onDiscover,
}: RootsPanelProps) {
  return (
    <section className="roots">
      <h2>Roots</h2>
      <ul>
        {roots.map((root) => (
          <li key={root}>
            <span>{root}</span>
            <button type="button" onClick={() => onRemove(root)}>
              Remove
            </button>
          </li>
        ))}
      </ul>
      <div className="actions">
        <input
          value={newRoot}
          onChange={(e) => onNewRootChange(e.target.value)}
          placeholder="C:\github\..."
          aria-label="new root"
        />
        <button
          type="button"
          disabled={newRoot.trim() === "" || busy}
          onClick={onAdd}
        >
          Add root
        </button>
        <button type="button" disabled={discovering} onClick={onDiscover}>
          {discovering ? "Discovering…" : "Discover"}
        </button>
      </div>
    </section>
  );
}
