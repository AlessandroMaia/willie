/* A tiny, self-contained relative-time label. `created_at`/`started_at`
 * are whole-second epoch strings (the daemon's format, same as jobs), so
 * parsing as seconds — not a calendar date — is what actually matches
 * the wire. */
export function relativeTime(epochSeconds: string): string {
  const started = Number(epochSeconds) * 1000;
  if (!Number.isFinite(started)) return "unknown";
  const diffSeconds = Math.floor((Date.now() - started) / 1000);
  if (diffSeconds < 60) return "just now";
  const minutes = Math.floor(diffSeconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.floor(hours / 24);
  return `${days} d ago`;
}
