import type { Change, ChangeKind } from "@/lib/proto";

/** The fragments the editor offers as a plain-text box: the three whose
 * wire name needs nothing beyond itself (`Fragment::parse`'s
 * `settings`/`instructions`/`mcp` arms). `rules/<file>` and
 * `hooks/<file>` are valid fragment names too, but editing one needs a
 * file picker this panel does not build yet. */
export interface FragmentDescriptor {
  id: "settings" | "instructions" | "mcp";
  label: string;
}

export const EDITABLE_FRAGMENTS: readonly FragmentDescriptor[] = [
  { id: "settings", label: "Settings" },
  { id: "instructions", label: "Instructions" },
  { id: "mcp", label: "MCP servers" },
];

/** How many of a `profile.check`/`profile.apply` change list fall into
 * each kind. A fixed key for every kind, zero-filled, so a caller never
 * has to guard a missing key for a kind the list happens not to use. */
export function summarizeChanges(
  changes: readonly Change[],
): Record<ChangeKind, number> {
  const counts: Record<ChangeKind, number> = {
    create: 0,
    merge: 0,
    overwrite: 0,
  };
  for (const change of changes) counts[change.kind] += 1;
  return counts;
}
