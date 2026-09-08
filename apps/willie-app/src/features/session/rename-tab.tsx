import { useEffect, useRef, useState } from "react";
import { TONE_DOT } from "@/components/tone";
import { Input } from "@/components/ui/input";
import { sessionName } from "@/lib/domain/sessions";
import type { Session } from "@/lib/proto";
import { cn } from "@/lib/utils";

interface RenameTabProps {
  session: Session;
  active: boolean;
  onSelect: (id: string) => void;
  onRename: (id: string, label: string | null) => void;
}

/**
 * One agent tab, a double-click away from renaming it in place. While
 * editing, the Input takes the tab's own slot instead of nesting
 * inside its button — tab selection is irrelevant mid-edit, and a
 * button may not validly contain another interactive element anyway.
 * The Input is seeded with `sessionName` and focused with its text
 * selected. Enter (or losing focus) commits through `onRename`, unless
 * the trimmed value is unchanged, which is a no-op; Escape discards
 * the edit without calling it. A settled guard makes sure Enter's own
 * commit and the blur that follows it as the input unmounts never fire
 * the bridge twice.
 */
export function RenameTab({
  session,
  active,
  onSelect,
  onRename,
}: RenameTabProps) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  const settledRef = useRef(false);

  useEffect(() => {
    if (editing) inputRef.current?.select();
  }, [editing]);

  function startEditing(): void {
    /* A double-click is also how a user selects a word while editing
     * (e.g. to retype it) — without this guard, that ordinary gesture
     * bubbles here and re-seeds `value`, wiping what was typed. */
    if (editing) return;
    settledRef.current = false;
    setValue(sessionName(session));
    setEditing(true);
  }

  function commit(): void {
    if (settledRef.current) return;
    settledRef.current = true;
    setEditing(false);
    const trimmed = value.trim();
    if (trimmed === sessionName(session)) return;
    onRename(session.id, trimmed || null);
  }

  function cancel(): void {
    settledRef.current = true;
    setEditing(false);
  }

  if (editing) {
    return (
      <Input
        ref={inputRef}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onClick={(e) => e.stopPropagation()}
        onDoubleClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit();
          else if (e.key === "Escape") cancel();
        }}
        onBlur={commit}
        className="h-7 w-28 px-1.5 py-0"
      />
    );
  }

  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      onClick={() => onSelect(session.id)}
      onDoubleClick={startEditing}
      className={cn(
        "flex items-center gap-1.5 whitespace-nowrap rounded-md px-2.5 py-1 text-sm",
        active
          ? "bg-muted text-foreground"
          : "text-muted-foreground hover:bg-muted/50",
      )}
    >
      <span className={cn("size-1.5 shrink-0 rounded-full", TONE_DOT.ok)} />
      {sessionName(session)}
    </button>
  );
}
