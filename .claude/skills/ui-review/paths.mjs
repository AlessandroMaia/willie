/* Where the harness reads from and writes to. The output lives under
 * `target/`, which is already ignored: screenshots are build output,
 * never versioned. */
import { mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const SKILL_DIR = dirname(fileURLToPath(import.meta.url));
export const ROOT = join(SKILL_DIR, "..", "..", "..");
export const OUT = join(ROOT, "target", "ui-shots");
export const BRIDGE = join(SKILL_DIR, "bridge.js");
export const APP_URL = process.env.WILLIE_UI_URL ?? "http://localhost:1420";

/** A browser profile per caller, so two scripts can run at once. */
export function profileFor(name) {
  const dir = join(OUT, ".browser", name);
  mkdirSync(dir, { recursive: true });

  return dir;
}

export function outDir(...parts) {
  const dir = join(OUT, ...parts);
  mkdirSync(dir, { recursive: true });

  return dir;
}

/** Fails closed with the one thing the caller has to do first. */
export async function requireDevServer() {
  try {
    const res = await fetch(APP_URL, { signal: AbortSignal.timeout(2500) });
    if (res.ok) return;
  } catch {
    /* falls through to the same message */
  }

  throw new Error(
    `no frontend at ${APP_URL} — run \`just ui-dev\` in another terminal ` +
      "(or `just dev` for the full app), then run this again",
  );
}
