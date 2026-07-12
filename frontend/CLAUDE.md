# Frontend

SvelteKit (Svelte 5 runes) + Vite, pure SPA (`adapter-static`, `ssr = false`,
output to `dist/`). Consumes halo-design tokens via `src/lib/styles/halo.css`
(copied verbatim); `--halo-*` vars in scoped `<style>` blocks. See the
`nib-design` skill for the brand delta.

## Structure

```
src/lib/model/      pure-TS document model: types, geometry, path <-> d walker,
                    document parse/serialize (svg-pathdata + DOMParser), shapes (ellipse)
src/lib/snap/       snap engine (nearest anchor, close-loop detection, grid)
src/lib/canvas/     gesture statechart (XState) — idle / panning / dragging
src/lib/tools/      pluggable editing tools + hit-testing (select / pen / circle / add / delete)
src/lib/workspace/  File System Access API wrappers + fallbacks
src/lib/stores/     rune stores: document (+history), viewport, tool, workspace, interaction
src/lib/components/ EditorCanvas, Overlay, ToolRail, Inspector, ColorInput, TopBar, SourceView, FileList, ImportDialog, Wordmark
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
- EditorCanvas imports the artwork imperatively into a `<g>` it owns (that's why
  `svelte/no-dom-manipulating` is disabled at those two lines).
