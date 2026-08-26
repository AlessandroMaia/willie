/** Formats an application version for display; `null` means unknown. */
export function formatVersion(version: string | null): string {
  return version === null ? "version unavailable" : `v${version}`;
}
