// PreToolUse hook: keeps unversioned local notes out of an agent's context
// unless the user explicitly allowed it in the current conversation.
//
// Allowed when either holds:
//   - WILLIE_ALLOW_LOCAL_DOCS=1 is set in the environment, or
//   - `.claude/.allow-local-docs` exists and was touched < 30 minutes ago.
// Otherwise any Read/Grep/Glob/Bash input that points into a protected
// directory is blocked (exit 2) with an instruction to ask the user first.
import { readFileSync, statSync } from "node:fs";
import { resolve } from "node:path";

const PROTECTED = ["docs/blueprint"];
const MARKER = ".claude/.allow-local-docs";
const TTL_MS = 30 * 60 * 1000;

let input;
try {
  input = JSON.parse(readFileSync(0, "utf8"));
} catch (error) {
  // Malformed input means the harness contract changed; report and let the
  // tool run rather than blocking every call.
  process.stderr.write(`guard-local-docs: unreadable hook input: ${error}\n`);
  process.exit(0);
}
const root = process.env.CLAUDE_PROJECT_DIR ?? input.cwd ?? process.cwd();

if (process.env.WILLIE_ALLOW_LOCAL_DOCS === "1" || markerIsFresh(root)) {
  process.exit(0);
}

const hit = candidates(input).find(isProtected);
if (hit === undefined) {
  process.exit(0);
}

process.stderr.write(
  `Blocked: "${hit}" points into an unversioned local-notes directory ` +
    `(${PROTECTED.join(", ")}). Do not read it unless the user explicitly ` +
    `asks in this conversation. If they did, create the marker file ` +
    `${MARKER} (valid for 30 minutes) and retry.\n`,
);
process.exit(2);

function markerIsFresh(dir) {
  try {
    const st = statSync(resolve(dir, MARKER));
    return Date.now() - st.mtimeMs < TTL_MS;
  } catch {
    return false;
  }
}

function candidates(inp) {
  const t = inp.tool_input ?? {};
  const keys = ["file_path", "path", "pattern", "command", "notebook_path"];
  return keys.map((k) => t[k]).filter((v) => typeof v === "string");
}

function isProtected(value) {
  const norm = value.replace(/\\/g, "/").toLowerCase();
  return PROTECTED.some((p) => norm.includes(p));
}
