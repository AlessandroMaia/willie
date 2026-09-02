import { XIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Item,
  ItemActions,
  ItemContent,
  ItemGroup,
} from "@/components/ui/item";
import { Spinner } from "@/components/ui/spinner";

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
    <section className="flex flex-col gap-2">
      <h2 className="font-medium text-muted-foreground text-sm">Roots</h2>

      {roots.length > 0 && (
        <ItemGroup className="gap-1">
          {roots.map((root) => (
            <Item key={root} size="xs" variant="muted">
              <ItemContent className="font-mono text-xs">{root}</ItemContent>
              <ItemActions>
                <Button
                  size="icon-xs"
                  variant="ghost"
                  aria-label={`Remove root ${root}`}
                  onClick={() => onRemove(root)}
                >
                  <XIcon />
                </Button>
              </ItemActions>
            </Item>
          ))}
        </ItemGroup>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <Input
          value={newRoot}
          onChange={(e) => onNewRootChange(e.target.value)}
          placeholder="C:\github\..."
          aria-label="new root"
          className="w-72"
        />
        <Button
          variant="outline"
          disabled={newRoot.trim() === "" || busy}
          onClick={onAdd}
        >
          Add root
        </Button>
        <Button variant="outline" disabled={discovering} onClick={onDiscover}>
          {discovering && <Spinner />}
          {discovering ? "Discovering…" : "Discover"}
        </Button>
      </div>
    </section>
  );
}
