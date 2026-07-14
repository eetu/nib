# Frontend

SvelteKit (Svelte 5 runes) + Vite, pure SPA (`adapter-static`, `ssr = false`,
output to `dist/`). Consumes halo-design tokens via `src/lib/styles/halo.css`
(adopted from the canonical file, with one deliberate deviation — `data-theme`
instead of a media query; see Notes); `--halo-*` vars in scoped `<style>`
blocks. See the `nib-design` skill for the brand delta.

## Structure

```
src/lib/core/       thin wrapper around the nib-core WASM engine (one-time init +
                    the Editor handle); the authoritative model/ops/geometry/undo
                    live in the Rust core (../core), not here
src/lib/model/      client view helpers over the WASM data contract: types (the TS
                    shape of the core's JSON), geometry math, pathToD +
                    nearestOnSubpath (render + hit-test), shapes (ellipse), STYLE_KEYS
src/lib/snap/       client snapping over the doc mirror (nearest anchor, close-loop, grid)
src/lib/canvas/     gesture statechart (XState) — idle / panning / dragging
src/lib/tools/      pluggable editing tools + hit-testing (select / pen / circle / add / delete)
src/lib/workspace/  File System Access API wrappers + fallbacks
src/lib/stores/     rune stores: document (a facade over the WASM Editor), viewport, tool, workspace, interaction, settings (theme + canvas bg)
src/lib/components/ EditorCanvas, Overlay, ToolRail, Inspector, ColorInput, TopBar, SourceView, FileList, ImportDialog, SettingsDialog, Wordmark
src/routes/         +layout (tokens + global control base), +page (composition root)
```

## Validation

Run `yarn validate` after changes — `svelte-check` (typecheck) + eslint + prettier.

- `yarn dev` — dev server (:5173)
- `yarn lint` / `yarn lint:fix`, `yarn format` / `yarn format:fix`
- `yarn typecheck` (svelte-check), `yarn test` (vitest, node/jsdom)
- `yarn build` — production build to `dist/`

Use yarn (the repo-vendored release). Unit tests cover the pure-TS core
(model / snap / edit geometry); component (browser) tests are Phase-0-deferred.

## Notes

- `svg-pathdata` v9: `new SVGPathData(d).toAbs().normalizeST().qtToC().aToC()`
  reduces any path to absolute M/L/H/V/C/Z before the walker builds anchor nodes.
- File System Access picker types aren't in this TS lib version — augmented in
  `src/lib/workspace/file-system-access.d.ts`.
- **Theme is `data-theme`-driven, not a media query** (so light/dark/auto can be
  chosen — see `stores/settings.svelte.ts` + `SettingsDialog`). `halo.css` is
  light-first (`:root`) + `[data-theme='dark']`; the root `+layout` resolves the
  mode to a concrete `data-theme` on `<html>` (re-resolving `auto` on system
  flips), and an inline script in `app.html` does the same pre-paint (no flash).
  `settings.canvasBg` (checker/light/dark) is the artwork's preview surface —
  absolute colours, orthogonal to the UI theme.
- EditorCanvas imports the artwork imperatively into a `<g>` it owns (that's why
  `svelte/no-dom-manipulating` is disabled at those two lines).
