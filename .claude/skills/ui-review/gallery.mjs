/* Renders the review page from the captured screenshots and one
 * findings file, inlining every image so the page is self-contained.
 *
 *   node .claude/skills/ui-review/gallery.mjs [findings.mjs]
 *
 * The findings file is this round's content, not tooling: it lives in
 * `target/ui-shots/` and is rewritten every pass. SKILL.md documents
 * its shape.
 */
import { readFileSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { OUT, SKILL_DIR } from "./paths.mjs";

const dataPath = resolve(process.argv[2] ?? join(OUT, "findings.mjs"));
const { FINDINGS, SCREENS, BRANCH } = await import(pathToFileURL(dataPath));

const out = join(OUT, "review.html");
const CHIP = {
  fixed: "fixed",
  open: "to fix",
  question: "question",
  clean: "clean",
};

const shot = (theme, id) =>
  `data:image/png;base64,${readFileSync(
    join(OUT, theme, `${id}.png`),
  ).toString("base64")}`;

function evidence(table) {
  if (!table) return "";

  const head = table.head.map((h) => `<th>${h}</th>`).join("");
  const rows = table.rows
    .map(
      (row) =>
        `<tr>${row
          .map((cell, i) => `<td${i ? ' class="num"' : ""}>${cell}</td>`)
          .join("")}</tr>`,
    )
    .join("");

  return `<div class="evidence"><table><thead><tr>${head}</tr></thead><tbody>${rows}</tbody></table></div>`;
}

const findings = FINDINGS.map(
  (f) => `<li class="finding" id="${f.id}" data-state="${f.state}">
  <div class="where"><a href="#${f.anchor}">${f.where}</a></div>
  <div>
    <h3>${f.title}</h3>
    <p>${f.body}</p>
    ${evidence(f.evidence)}
  </div>
  <span class="chip" data-state="${f.state}">${CHIP[f.state]}</span>
</li>`,
).join("\n");

const screens = SCREENS.map(
  (s) => `<article class="screen" id="${s.id}">
  <div class="screen-head">
    <span class="route">${s.route}</span>
    <span class="title">${s.title}</span>
    <div class="segmented" role="group" aria-label="Theme for ${s.route}">
      <button type="button" data-shot-theme="dark" aria-pressed="true">Dark</button>
      <button type="button" data-shot-theme="light" aria-pressed="false">Light</button>
    </div>
  </div>
  <figure>
    <img data-variant="dark" src="${shot("dark", s.id)}"
      alt="${s.title}, dark theme" width="1100" height="720">
    <img data-variant="light" src="${shot("light", s.id)}"
      alt="${s.title}, light theme" width="1100" height="720" hidden>
  </figure>
  <ul class="notes">
    ${s.notes
      .map(
        ([state, text]) =>
          `<li><span class="chip" data-state="${state}">${CHIP[state]}</span><span>${text}</span></li>`,
      )
      .join("\n    ")}
  </ul>
</article>`,
).join("\n");

const counts = FINDINGS.reduce((acc, f) => {
  acc[f.state] = (acc[f.state] ?? 0) + 1;

  return acc;
}, {});

const html = `<title>Willie Shell Review</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600&family=JetBrains+Mono:wght@400;500&display=swap">
<style>
${readFileSync(join(SKILL_DIR, "gallery.css"), "utf8")}
</style>

<div class="wrap">
  <header class="masthead">
    <p class="eyebrow">Branch <code>${BRANCH}</code> · captured from the running app</p>
    <h1>Willie Shell Review</h1>
    <p class="lede">Every screen of the desktop shell, in both themes, driven with
    fixture data through the real frontend. Point at what should change; the
    numbers under each finding come from measuring the live DOM, not from
    reading the screenshots.</p>
    <div class="facts">
      <div class="fact"><span>screens</span><b>${SCREENS.length}</b></div>
      <div class="fact"><span>themes</span><b>dark + light</b></div>
      <div class="fact"><span>viewport</span><b>1100 × 720</b></div>
      <div class="fact"><span>fixed</span><b>${counts.fixed ?? 0}</b></div>
      <div class="fact"><span>to fix</span><b>${counts.open ?? 0}</b></div>
      <div class="fact"><span>questions</span><b>${counts.question ?? 0}</b></div>
    </div>

    <ol class="loop">
      <li><b>Comment on a screen</b>Use the comment tool on the card you want changed, and send it to Claude so it reaches the session.</li>
      <li><b>I fix it on the branch</b>Root cause first, then a test that fails before the change, then <code>just check</code>.</li>
      <li><b>This page comes back updated</b>Same link, re-captured screenshots, the finding flipped to <em>fixed</em> with its measurement.</li>
      <li><b>You ask for the commit</b>Nothing is committed until you say so; the branch holds the work meanwhile.</li>
    </ol>
  </header>

  <section>
    <h2>Findings</h2>
    <p class="section-note">Ordered by how much they cost you on screen. Each links to the
    screen it lives on.</p>
    <ul class="findings">
${findings}
    </ul>
  </section>

  <section>
    <div class="toolbar">
      <p>Every screen below, at the window's minimum-plus size. Toggle a card to
      compare themes, or switch them all at once.</p>
      <div class="segmented" role="group" aria-label="Theme for every screen">
        <button type="button" data-all-theme="dark" aria-pressed="true">All dark</button>
        <button type="button" data-all-theme="light" aria-pressed="false">All light</button>
      </div>
    </div>
    <div class="screens">
${screens}
    </div>
  </section>

  <footer>
    <p>Captured through the app's own frontend, with the Tauri bridge answered by
    fixtures: three systems, four sessions (running, shell, exited, failed), a doctor
    with one failing check. The window chrome you see is the app's own frameless
    header, not the browser's.</p>
  </footer>
</div>

<script>
  function setTheme(scope, theme) {
    for (const img of scope.querySelectorAll("figure img")) {
      img.hidden = img.dataset.variant !== theme;
    }
    for (const btn of scope.querySelectorAll("[data-shot-theme]")) {
      btn.setAttribute("aria-pressed", String(btn.dataset.shotTheme === theme));
    }
  }

  document.addEventListener("click", (event) => {
    const one = event.target.closest("[data-shot-theme]");
    if (one) {
      setTheme(one.closest(".screen"), one.dataset.shotTheme);
      return;
    }

    const all = event.target.closest("[data-all-theme]");
    if (!all) return;

    const theme = all.dataset.allTheme;
    for (const screen of document.querySelectorAll(".screen")) setTheme(screen, theme);
    for (const btn of document.querySelectorAll("[data-all-theme]")) {
      btn.setAttribute("aria-pressed", String(btn.dataset.allTheme === theme));
    }
  });
</script>
`;

writeFileSync(out, html);
console.log(`${out}  ${(statSync(out).size / 1024 / 1024).toFixed(2)} MB`);
