/* A minimal Chrome DevTools Protocol driver: no dependencies, on the
 * global WebSocket and fetch that Node 22+ ships. Enough to open the
 * frontend, drive it, read the live DOM and capture it. */
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

/* Any Chromium speaks the protocol; Edge ships with Windows, so a
 * machine without Chrome still works. */
const BROWSERS = [
  process.env.WILLIE_UI_BROWSER,
  "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
  "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
  "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
  "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
].filter(Boolean);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export function browserPath() {
  const found = BROWSERS.find((path) => existsSync(path));

  if (!found) {
    throw new Error(
      "no Chromium browser found: install Chrome or Edge, or set " +
        "WILLIE_UI_BROWSER to a browser executable",
    );
  }

  return found;
}

export async function launch({
  port = 9333,
  profile,
  headless = true,
  hideScrollbars = true,
  size = "1100,720",
} = {}) {
  mkdirSync(profile, { recursive: true });

  const args = [
    `--remote-debugging-port=${port}`,
    `--user-data-dir=${profile}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-features=Translate,MediaRouter",
    `--window-size=${size}`,
    ...(hideScrollbars ? ["--hide-scrollbars"] : []),
    "about:blank",
  ];

  if (headless) args.unshift("--headless=new");

  const child = spawn(browserPath(), args, { stdio: "ignore" });

  let pages = null;

  for (let i = 0; i < 100; i++) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/json/list`);
      pages = (await res.json()).filter((t) => t.type === "page");
      if (pages.length) break;
    } catch {
      /* not listening yet */
    }
    await sleep(100);
  }

  if (!pages?.length) throw new Error("the browser never exposed a page");

  return { child, wsUrl: pages[0].webSocketDebuggerUrl, port };
}

export async function attach(wsUrl) {
  const ws = new WebSocket(wsUrl);

  await new Promise((resolve, reject) => {
    ws.addEventListener("open", resolve, { once: true });
    ws.addEventListener("error", reject, { once: true });
  });

  let id = 0;
  const pending = new Map();
  const waiters = [];
  const logs = [];

  ws.addEventListener("message", (msg) => {
    const data = JSON.parse(msg.data);

    if (data.id !== undefined) {
      const call = pending.get(data.id);
      pending.delete(data.id);
      if (!call) return;
      if (data.error) call.reject(new Error(JSON.stringify(data.error)));
      else call.resolve(data.result);

      return;
    }

    if (data.method === "Runtime.consoleAPICalled") {
      logs.push({
        level: data.params.type,
        text: data.params.args
          .map((a) => a.value ?? a.description ?? a.type)
          .join(" "),
      });
    }

    if (data.method === "Runtime.exceptionThrown") {
      const details = data.params.exceptionDetails;
      logs.push({
        level: "exception",
        text: details.exception?.description ?? details.text,
      });
    }

    if (data.method === "Log.entryAdded") {
      logs.push({
        level: data.params.entry.level,
        text: data.params.entry.text,
      });
    }

    for (const waiter of waiters.splice(0)) waiter(data);
  });

  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const callId = ++id;
      pending.set(callId, { resolve, reject });
      ws.send(JSON.stringify({ id: callId, method, params }));
    });

  await send("Page.enable");
  await send("Runtime.enable");
  await send("Log.enable");

  const page = {
    send,
    logs,
    close: () => ws.close(),

    /** Errors and exceptions only — the signal worth acting on. */
    errors: () =>
      logs.filter((l) => l.level === "error" || l.level === "exception"),

    async injectOnNewDocument(file) {
      await send("Page.addScriptToEvaluateOnNewDocument", {
        source: readFileSync(file, "utf8"),
      });
    },

    /** Drives the app's theme: it follows `prefers-color-scheme`. */
    async theme(value) {
      await send("Emulation.setEmulatedMedia", {
        features: [{ name: "prefers-color-scheme", value }],
      });
    },

    async viewport(width, height, scale = 1) {
      await send("Emulation.setDeviceMetricsOverride", {
        width,
        height,
        deviceScaleFactor: scale,
        mobile: false,
      });
    },

    async goto(url, settle = 900) {
      const loaded = new Promise((resolve) => {
        waiters.push(function self(event) {
          if (event.method === "Page.loadEventFired") resolve();
          else waiters.push(self);
        });
      });

      await send("Page.navigate", { url });
      await Promise.race([loaded, sleep(8000)]);
      await sleep(settle);
    },

    async eval(expression) {
      const res = await send("Runtime.evaluate", {
        expression,
        returnByValue: true,
        awaitPromise: true,
      });

      if (res.exceptionDetails) {
        throw new Error(
          res.exceptionDetails.exception?.description ??
            res.exceptionDetails.text,
        );
      }

      return res.result.value;
    },

    /** Clicks a selector, or the first element under it whose text
     * contains `text`. Returns false when nothing matched. */
    async click(selector, { text } = {}) {
      const pick = text
        ? `[...document.querySelectorAll(${JSON.stringify(selector)})]
             .find((e) => e.textContent.trim().includes(${JSON.stringify(text)}))`
        : `document.querySelector(${JSON.stringify(selector)})`;

      const clicked = await page.eval(`(() => {
        const el = ${pick};
        if (!el) return false;
        el.scrollIntoView({ block: "center" });
        el.click();
        return true;
      })()`);

      await sleep(450);

      return clicked;
    },

    async key(key, code, keyCode) {
      for (const type of ["keyDown", "keyUp"]) {
        await send("Input.dispatchKeyEvent", {
          type,
          key,
          code,
          windowsVirtualKeyCode: keyCode,
        });
      }

      await sleep(350);
    },

    async shot(dir, name) {
      const res = await send("Page.captureScreenshot", { format: "png" });
      const path = join(dir, `${name}.png`);
      writeFileSync(path, Buffer.from(res.data, "base64"));

      return path;
    },
  };

  return page;
}
