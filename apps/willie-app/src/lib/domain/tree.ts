import type { TreeEntry } from "@/lib/proto";

/**
 * Directories before files, then case-insensitive by name — the one
 * order every tree row list renders in, regardless of what order the
 * daemon happened to return the entries.
 */
export function sortEntries(entries: TreeEntry[]): TreeEntry[] {
  return [...entries].sort((a, b) => {
    if (a.kind !== b.kind) return a.kind === "dir" ? -1 : 1;
    return a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
  });
}

/**
 * Joins a workspace-relative directory and a child name into a
 * normalised relative path — no leading "./", no trailing "/" — so it
 * matches what the daemon expects verbatim and its git flags line up.
 */
export function joinPath(base: string, name: string): string {
  const cleanBase = base.replace(/\/+$/, "");
  const cleanName = name.replace(/^\.\/+/, "");
  return cleanBase === "" ? cleanName : `${cleanBase}/${cleanName}`;
}
