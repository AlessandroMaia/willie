/* The screens the harness knows about, in one place: `shots.mjs` and
 * `measure.mjs` both read it, and neither may import the other — they
 * are scripts that run on import. Mirrors `app/routes.ts`. */

/** The sidebar's four screens and the setup drawer's six entries. */
export const ROUTES = [
  ["session", "/session"],
  ["sandbox", "/sandbox"],
  ["profiles", "/profiles"],
  ["usage", "/usage"],
  ["setup-engine", "/setup/engine"],
  ["setup-tools", "/setup/tools"],
  ["setup-plugins", "/setup/plugins"],
  ["setup-profile-store", "/setup/profile-store"],
  ["setup-systems", "/setup/systems"],
  ["setup-settings", "/setup/settings"],
];

/** States no URL reaches, each opened from the Session screen. */
export const STATES = [
  ["drawer-setup", (page) => page.click('[aria-label="Open setup"]')],
  [
    "workspace-panel",
    async (page) => {
      await page.key("Escape", "Escape", 27);

      return page.click('[aria-label="Workspace panel"]');
    },
  ],
  [
    "system-selector",
    async (page) => {
      await page.key("Escape", "Escape", 27);

      return page.click('[aria-label="Switch system"]');
    },
  ],
  [
    "sidebar-collapsed",
    async (page) => {
      await page.key("Escape", "Escape", 27);

      return page.click('[aria-label="Toggle sidebar"]');
    },
  ],
];
