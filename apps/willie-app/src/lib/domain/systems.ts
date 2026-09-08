import type { Project } from "@/lib/proto";

/**
 * The system every screen is scoped to: the persisted preference if it
 * still names a project, else the first one, else none. A project list
 * can arrive empty before the daemon reports anything, and a preferred
 * id can outlive the project it named (removed, or never seen yet).
 */
export function resolveCurrent(
  preferred: string | null | undefined,
  projects: Project[],
): Project | null {
  if (preferred) {
    const found = projects.find((project) => project.id === preferred);
    if (found) return found;
  }

  return projects[0] ?? null;
}

/**
 * Two letters standing in for a system with no picture: the initials
 * of its first two words, or the first two characters when it is one
 * word.
 */
export function glyphFor(name: string): string {
  const [first, second] = name.trim().split(/\s+/).filter(Boolean);
  const glyph =
    first !== undefined && second !== undefined
      ? first.charAt(0) + second.charAt(0)
      : (first ?? name).slice(0, 2);

  return glyph.toUpperCase();
}
