import { relativeTime } from "@/components/relative-time";
import { StatusBadge } from "@/components/status-badge";
import {
  Item,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemTitle,
} from "@/components/ui/item";
import type { DenialRow } from "@/lib/domain/sandbox";
import { explain } from "@/lib/domain/sandbox";
import { sessionName } from "@/lib/domain/sessions";
import type { Session } from "@/lib/proto";

interface DenialHistoryProps {
  rows: DenialRow[];
  sessions: Session[];
}

/**
 * The system's denial history, one row per class/name/session, in the
 * chronological order `rows` already carries (newest `lastAt` first).
 * No action lives on a row: a denial is a fact the sandbox recorded,
 * never something a click here can grant — capability changes belong
 * to the drawer alone.
 */
export function DenialHistory({ rows, sessions }: DenialHistoryProps) {
  function nameFor(sessionId: string): string {
    const session = sessions.find((s) => s.id === sessionId);
    return session ? sessionName(session) : sessionId;
  }

  return (
    <ItemGroup className="gap-1">
      {rows.map((row) => (
        <Item
          key={`${row.sessionId}-${row.class}-${row.name}-${row.lastAt}`}
          variant="outline"
          size="sm"
        >
          <ItemContent>
            <ItemTitle>
              <StatusBadge tone="muted">{row.class}</StatusBadge>
              {row.name}
            </ItemTitle>
            <ItemDescription>{explain(row.class, row.name)}</ItemDescription>
          </ItemContent>
          <div className="flex flex-col items-end gap-0.5 text-muted-foreground text-xs">
            <span>{nameFor(row.sessionId)}</span>
            <span>{relativeTime(row.lastAt)}</span>
          </div>
          <StatusBadge tone="warning">×{row.count}</StatusBadge>
        </Item>
      ))}
    </ItemGroup>
  );
}
