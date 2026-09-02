import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Field, FieldLabel } from "@/components/ui/field";
import { Item, ItemContent, ItemGroup } from "@/components/ui/item";
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
    <section className="flex flex-col gap-2">
      <h2 className="font-medium text-muted-foreground text-sm">Discovered</h2>

      {candidates.length === 0 ? (
        <p className="text-muted-foreground text-sm">
          No repositories found under the roots.
        </p>
      ) : (
        <>
          <ItemGroup className="gap-1">
            {candidates.map((c) => {
              const id = `candidate-${c.path}`;
              return (
                <Item key={c.path} size="xs" variant="muted">
                  <ItemContent>
                    <Field orientation="horizontal">
                      <Checkbox
                        id={id}
                        checked={selected.has(c.path)}
                        onCheckedChange={() => onToggle(c.path)}
                      />
                      <FieldLabel htmlFor={id}>
                        {c.name}
                        <span className="font-mono text-muted-foreground text-xs">
                          {c.path}
                        </span>
                      </FieldLabel>
                    </Field>
                  </ItemContent>
                </Item>
              );
            })}
          </ItemGroup>
          <div>
            <Button
              disabled={selected.size === 0 || busy}
              onClick={onAddSelected}
            >
              Add selected
            </Button>
          </div>
        </>
      )}
    </section>
  );
}
