/* Audits window-level overflow: the app window must never scroll —
 * the header and the status bar are fixed rows and each screen scrolls
 * inside the centre pane. Anything reported here breaks that.
 *
 *   node .claude/skills/ui-review/measure.mjs
 */
import { attach, launch } from "./cdp.mjs";
import { APP_URL, BRIDGE, profileFor, requireDevServer } from "./paths.mjs";
import { ROUTES } from "./routes.mjs";

const PROBE = `(() => {
  const de = document.documentElement;
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const overflowing = [];

  for (const el of document.querySelectorAll("body *")) {
    const r = el.getBoundingClientRect();
    if (r.width === 0 && r.height === 0) continue;
    if (r.bottom > vh + 0.5 || r.right > vw + 0.5) {
      overflowing.push({
        tag: el.tagName.toLowerCase(),
        slot: el.dataset.slot ?? "",
        cls: el.className.toString().slice(0, 70),
        rect: [r.x, r.y, r.width, r.height].map(Math.round),
        bottom: Math.round(r.bottom),
        right: Math.round(r.right),
      });
    }
  }

  return {
    viewport: [vw, vh],
    doc: [de.scrollWidth, de.scrollHeight, de.clientWidth, de.clientHeight],
    scrolls: de.scrollHeight > de.clientHeight || de.scrollWidth > de.clientWidth,
    overflowing: overflowing.slice(0, 8),
  };
})()`;

await requireDevServer();

const { wsUrl } = await launch({
  profile: profileFor("measure"),
  port: 9336,
  hideScrollbars: false,
});

const page = await attach(wsUrl);
await page.injectOnNewDocument(BRIDGE);
await page.viewport(1100, 720, 1);

let scrolling = 0;

for (const [, route] of ROUTES) {
  for (const withPanel of [false, true]) {
    await page.goto("about:blank", 50);
    await page.goto(`${APP_URL}/#${route}`, 1100);
    if (withPanel) await page.click('[aria-label="Workspace panel"]');

    const m = await page.eval(PROBE);
    if (m.scrolls) scrolling += 1;

    console.log(
      `${m.scrolls ? "SCROLLS" : "ok     "} ${route.padEnd(22)} ` +
        `${withPanel ? "panel open " : "panel shut "}` +
        `doc(w,h,cw,ch)=${m.doc.join(",")}`,
    );

  /* Only what the window itself cannot contain: content taller than the
   * centre pane is clipped by the scroll area and is not a defect. */
    if (m.scrolls) {
      for (const el of m.overflowing) {
        console.log(
          `        <${el.tag}${el.slot ? ` slot=${el.slot}` : ""}> ` +
            `rect=${el.rect.join(",")} bottom=${el.bottom} right=${el.right}`,
        );
        console.log(`          class="${el.cls}"`);
      }
    }
  }
}

page.close();
process.exit(scrolling === 0 ? 0 : 1);
