// App-level display preferences as a shared rune store — the Svelte equivalent
// of a jotai atom: import `settings` anywhere and read/write it directly, no
// prop-drilling. Holds cross-cutting view prefs (theme, canvas backdrop); tool
// prefs (snap/grid) live in `tool.svelte.ts`, the document session in
// `document.svelte.ts`.
//
// `themeMode` is the chosen preference; the *resolved* effective theme
// ('light'|'dark') is applied as `data-theme` on <html> by the root layout (and
// pre-paint by the inline shell script in app.html) so the halo tokens key off
// it. `canvasBg` is orthogonal to the UI theme — it's the surface the artwork
// is previewed against, so its "light"/"dark" are absolute, not theme-following.
//
// Persistence is explicit in the setters: a module-level rune store has no
// component/effect context to run an $effect in.

export type ThemeMode = "auto" | "light" | "dark";
export type CanvasBg = "checker" | "light" | "dark";
/** UI density preference: `basic` surfaces only touch-up tools (select/node-edit/style/
 *  save); `advanced` shows the full pro surface (shapes, path craft, booleans, gradients,
 *  group tree). Gates chrome only — the engine + LLM/MCP op surface are always full. */
export type UiLevel = "basic" | "advanced";

const THEME_KEY = "nib:theme";
const BG_KEY = "nib:canvasBg";
const UI_LEVEL_KEY = "nib:uiLevel";
const BACKEND_URL_KEY = "nib:backendUrl";
const BACKEND_TOKEN_KEY = "nib:backendToken";
/** Dev default so connected mode + a local MCP client work out of the box (matches the backend's
 *  seeded developer token). Overridable in Settings. */
const DEV_TOKEN = "nib-dev-token";

function read(key: string): string | null {
  return typeof localStorage !== "undefined" ? localStorage.getItem(key) : null;
}

function initialMode(): ThemeMode {
  const v = read(THEME_KEY);
  return v === "light" || v === "dark" || v === "auto" ? v : "auto";
}

function initialBg(): CanvasBg {
  const v = read(BG_KEY);
  return v === "light" || v === "dark" || v === "checker" ? v : "checker";
}

function initialUiLevel(): UiLevel {
  // Default to the full surface (no capability hidden on first run — "not a toy"); basic is
  // the deliberate opt-in that declutters down to touch-ups.
  const v = read(UI_LEVEL_KEY);
  return v === "basic" || v === "advanced" ? v : "advanced";
}

export const settings = $state<{
  themeMode: ThemeMode;
  canvasBg: CanvasBg;
  uiLevel: UiLevel;
  /** Whether the user has ever *explicitly* picked a UI level. False = first run, so the
   *  composition root shows the one-time interface chooser; after any pick it's Settings-only. */
  uiLevelChosen: boolean;
  /** Backend base URL for connected mode; "" = same origin (dev proxy / backend-embedded build). */
  backendUrl: string;
  /** The user's bearer token — sent to the backend + pasted into an MCP client. */
  backendToken: string;
}>({
  themeMode: initialMode(),
  canvasBg: initialBg(),
  uiLevel: initialUiLevel(),
  uiLevelChosen: read(UI_LEVEL_KEY) !== null,
  backendUrl: read(BACKEND_URL_KEY) ?? "",
  backendToken: read(BACKEND_TOKEN_KEY) ?? DEV_TOKEN,
});

function persist(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* storage unavailable — non-fatal */
  }
}

export function setThemeMode(mode: ThemeMode) {
  settings.themeMode = mode;
  persist(THEME_KEY, mode);
}

export function setCanvasBg(bg: CanvasBg) {
  settings.canvasBg = bg;
  persist(BG_KEY, bg);
}

export function setUiLevel(level: UiLevel) {
  settings.uiLevel = level;
  // Persisting the key is itself the "chosen" signal (initialUiLevelChosen reads its presence),
  // so this both records the level and retires the first-run chooser for good.
  settings.uiLevelChosen = true;
  persist(UI_LEVEL_KEY, level);
}

export function setBackendUrl(url: string) {
  settings.backendUrl = url.trim();
  persist(BACKEND_URL_KEY, settings.backendUrl);
}

export function setBackendToken(token: string) {
  settings.backendToken = token.trim();
  persist(BACKEND_TOKEN_KEY, settings.backendToken);
}
