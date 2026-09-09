/* Reads the live DOM instead of guessing from a screenshot: geometry
 * and markup for whatever a selector matches, on one route.
 *
 *   node .claude/skills/ui-review/probe.mjs /session '[data-slot="status-bar"]'
 *
 * Pass `--collapse` to click the sidebar toggle before measuring.
 */
import { attach, launch } from "./cdp.mjs";
import { APP_URL, BRIDGE, profileFor, requireDevServer } from "./paths.mjs";

const args = process.argv.slice(2).filter((a) => a !== "--collapse");
const collapse = process.argv.includes("--collapse");
const route = args[0] ?? "/session";
const selector = args[1] ?? "body";

await requireDevServer();

const { wsUrl } = await launch({ profile: profileFor("probe"), port: 9335 });
const page = await attach(wsUrl);

await page.injectOnNewDocument(BRIDGE);
await page.viewport(1100, 720, 1);
await page.goto(`${APP_URL}/#${route}`, 1200);

if (collapse) await page.click('[aria-label="Toggle sidebar"]');

const found = await page.eval(`(() => {
  const els = [...document.querySelectorAll(${JSON.stringify(selector)})];

  return els.slice(0, 6).map((el) => {
    const r = el.getBoundingClientRect();

    return {
      rect: {
        left: Math.round(r.left),
        top: Math.round(r.top),
        right: Math.round(r.right),
        bottom: Math.round(r.bottom),
        width: Math.round(r.width),
        height: Math.round(r.height),
      },
      text: el.innerText?.slice(0, 300) ?? "",
      html: el.outerHTML.slice(0, 1200),
    };
  });
})()`);

console.log(`${found.length} match(es) for ${selector} on #${route}\n`);
console.log(JSON.stringify(found, null, 2));

const errors = page.errors();
if (errors.length) {
  console.log(`\nconsole errors: ${errors.length}`);
  for (const error of errors.slice(0, 5)) {
    console.log(`  [${error.level}] ${error.text.split("\n")[0]}`);
  }
}

page.close();
process.exit(0);
