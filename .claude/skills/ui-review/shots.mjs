/* Captures every screen in both themes, plus the states no URL
 * reaches. Run through `just ui-shots`. */
import { join } from "node:path";
import { attach, launch } from "./cdp.mjs";
import {
  APP_URL,
  BRIDGE,
  OUT,
  outDir,
  profileFor,
  requireDevServer,
} from "./paths.mjs";
import { ROUTES, STATES } from "./routes.mjs";

await requireDevServer();

const { wsUrl } = await launch({ profile: profileFor("shots") });
const page = await attach(wsUrl);

await page.injectOnNewDocument(BRIDGE);
await page.viewport(1100, 720, 1);

for (const theme of ["dark", "light"]) {
  const dir = outDir(theme);
  await page.theme(theme);

  for (const [name, route] of ROUTES) {
    /* Route to route is a hash change, not a reload: without a fresh
     * document the previous screen's React state (a collapsed sidebar,
     * an open drawer) leaks into the next shot. */
    await page.goto("about:blank", 50);
    await page.goto(`${APP_URL}/#${route}`, 1200);
    await page.shot(dir, name);
    console.log(`${theme.padEnd(5)} ${name}`);
  }

  await page.goto("about:blank", 50);
  await page.goto(`${APP_URL}/#/session`, 1200);

  for (const [name, open] of STATES) {
    await open(page);
    await page.shot(dir, name);
    console.log(`${theme.padEnd(5)} ${name}`);
  }
}

const errors = page.errors();
console.log(`\n${join(OUT, "<theme>")}`);
console.log(`console errors: ${errors.length}`);
for (const error of errors.slice(0, 10)) {
  console.log(`  [${error.level}] ${error.text.split("\n")[0]}`);
}

page.close();
process.exit(0);
