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

const THEME_KEY = "nib:theme";
const BG_KEY = "nib:canvasBg";

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

export const settings = $state<{ themeMode: ThemeMode; canvasBg: CanvasBg }>({
  themeMode: initialMode(),
  canvasBg: initialBg(),
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
