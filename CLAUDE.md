# nib — repo overview

A direct-manipulation SVG **path editor**: paste/open an LLM-generated SVG, drag
its anchor points and bezier handles, snap endpoints together to close loops,
save back. The LLM roughs out the shape; nib does the last-5% by-hand tuning
that prose can't. Sibling in eetu's homebrew family ([halo](../halo),
[ocular](../ocular), [scribe](../scribe)) — shares the halo-design system.

**A Rust/WASM core + a SvelteKit SPA.** The editing engine — document model,
operation vocabulary + reducer, geometry, parse/serialize, snapping, undo history
— lives in a Rust crate (`core/`, `nib-core`) compiled to **WASM**. The SPA is the
view + interaction layer over it: the `document` store is a thin facade that drives
the WASM `Editor` with **ops** and mirrors its `state()` back into Svelte runes.
One engine, so the *same* core runs native on a later backend. Today nib is still
fully client-side: it reads/writes the user's own files via the File System Access
API, and the **live demo** is a static build on **GitHub Pages**
(`.github/workflows/pages.yaml` → https://eetu.github.io/nib/) with
`fallback: index.html` (the family backend contract). A backend — persistence +
realtime sync + an MCP tool surface, all running the same core — is a planned,
*additive* track (see the roadmap); the SPA stays the editor, never a place logic
migrates out of.

## Layout

```
core/             Rust nib-core engine → WASM (browser) + native (later backend):
                  model, ops + reducer, geometry, parse/serialize, snap, undo
frontend/         SvelteKit (Svelte 5 runes) + Vite SPA, adapter-static → dist/;
                  consumes core/pkg (wasm-pack output) via a link: dep
.claude/skills/   nib-design skill (glyph, wordmark, layout, voice)
justfile          task runner (just dev / build / validate / test / test-e2e)
```

Per-area detail in `frontend/CLAUDE.md`.

## Conventions (the load-bearing invariants)

- **Model + ops + geometry live in the Rust core** (`core/src`, `nib-core`),
  compiled to WASM. Paths normalize to absolute cubic-bezier anchor nodes
  (M/L/H/V/C/S/Q/T/A fold in via `kurbo::BezPath::from_svg`, quads elevated to
  cubics); every edit is an `Op` applied by a pure reducer, and the Svelte
  `document` store is a thin facade over the WASM `Editor`. The TS unit tests were
  ported to `cargo test` as the parity oracle. The parallel TS *engine*
  (parse/serialize/edit) is gone; `frontend/src/lib/model` + `snap` keep only the
  pure client render + hit-test + snap helpers the canvas/tools need locally, plus
  the TS type contract for the untyped WASM boundary — the authoritative engine is
  the Rust core.
- **Byte-for-byte preservation.** The original SVG source is kept; on export
  only *edited* paths get their `d` re-serialized (spliced in place). Everything
  else — other elements, attributes, unedited paths — is preserved verbatim.
  Arcs in an *edited* path convert to cubics (lossy); untouched paths never change.
- **Added (drawn) paths** carry `added: true` + their own `attributes` and have
  no source location. They render from the model (a Svelte-managed `<g class=
  "drawn">`, not the imperatively-imported artwork) and are *appended* before
  `</svg>` on export. Everything else treats them like any path (editable,
  snappable, undoable, persisted).
- **Layers = objects + groups (Figma/Pixelmator model, one level).** The single
  Inspector **LAYERS** panel *is* the object list: every path/shape is a row,
  **z-order = the `paths` array order** (drag a row to reorder → `ReorderPath`),
  and a contiguous run of paths sharing a `layer` id renders as a collapsible
  **group**. **Group** (`GroupPaths` op) wraps the selection into a named group —
  a `<g id="name">` on export — pulling its members contiguous; **Ungroup**
  (`deleteLayer`) dissolves it (geometry untouched). Per-path (`PathElement.hidden`
  + `SetPathHidden`) and per-group (`Layer.visible`) **show/hide** export as
  `display="none"`. New shapes land on `activeLayer`. **The panel's z-order matches
  the canvas *and* the export**: drawn paths append in array order (wrapping
  contiguous group runs in `<g>`), imported paths fill their source `<path>` slots
  in draw order. **Export is byte-for-byte until something is reordered/grouped/
  edited/hidden** — grouping + reordering is the first *active re-serialization of
  structure*. Core: `Layer`/`SvgDocument.layers`/`activeLayer` + `PathElement.layer`
  +`hidden`; ops `groupPaths`/`setPathHidden`/`reorderPath`/`setLayerVisible`/
  `renameLayer`/`deleteLayer`. **Caveats (Phase D):** groups are one level (no
  groups-in-groups yet); imported paths group for membership/visibility/order but
  aren't `<g>`-wrapped on export (they stay in their source slots), and drawn
  shapes render in a group above all imported paths — full interleaving + nesting
  needs the Phase-D object tree + stable ids.
- **Two coordinate systems in the canvas.** Artwork is drawn in a scaled `<g>`
  (document units); the editing overlay is drawn in screen space so handles stay
  a constant pixel size at any zoom. `viewport.toScreen/toDoc` bridge them.
- **Tools are a pluggable seam** (`frontend/src/lib/tools`): each is
  `{ id, cursor, begin() → DragSession?, hover?() }`. Add a tool = add a module +
  registry entry; the rail groups them (`ToolRail`: select · create · nodes) so
  it stays scannable as they grow. `hover()` drives live aids (pen rubber-band,
  circle centre-snap). Tools: select/move, pen (draw new paths — and *resume*
  an open subpath by clicking either endpoint; grabbing the head reverses the
  subpath so appends still run off the tail, via `editor.reverseSubpath`),
  circle (drag out a closed 4-node bezier), add-node, delete-node. Shapes are built as
  editable paths (`model/shapes.ts`), not native `<circle>`/`<rect>`.
- **Selection = node + path.** `selection` is the active node; `selectedPath` is
  an explicit path selection (PATHS row / path-body click). `selectedPathIndex`
  is the effective selected path: the selected node's path if any, else
  `selectedPath`. The STYLE panel targets it.
- **Object vs node mode (one tool, like Figma), switched by double-click.** The
  select tool defaults to **object mode**: clicking a path selects it
  (`objectSelected` = a path selected with *no* node *and* not node-editing) and
  the Overlay draws the **transform box** — a dashed box (padded, `SELECT_PAD_PX`)
  + **8 resize handles** + a **rotate knob** above the top-centre + an accent
  **centerline** (light casing + accent core so it reads on any stroke). Drag a
  corner (both axes) / edge (one axis) to scale about the opposite anchor (shift
  keeps aspect), drag the knob to rotate about the box centre (shift → 15° steps),
  drag the body to move the whole shape. **Anchors are hidden and not hit-tested
  in object mode**, so a drag unambiguously moves the shape — crucial when zoomed
  out and nodes cluster. **Double-clicking a shape enters node mode**
  (`document.nodeEditIndex`): its anchors + handles appear and become editable and
  the transform box hides; Esc, a tool switch, or clicking empty returns to object
  mode. Non-select tools (pen/add-node/delete-node) always show + hit anchors.
  Hit-testing (`lib/tools/{hit,transform}.ts` + the `transform`/`rotate` Hit kinds
  + `select`'s scale/rotate/pathDrag) gates anchors/handles on this mode, so a
  path's own nodes are never shadowed by transform handles. Deleting the last node
  soft-deletes the now-empty path.
- **Styling.** Drawn/shape paths (`added`) carry an `attributes` map the STYLE
  panel edits directly (fill/stroke/width/opacity). New paths are stamped with
  `tools.newStyle` at creation, editable up front: with a create tool active and
  nothing selected, the panel becomes "new shape style" and edits those defaults
  (persisted). Imported paths edit a
  `styleOverride` map, merged over their parsed `attributes` for display and
  spliced into the source `<path>` tag on export (`withAttr`) so everything
  else in the tag stays byte-for-byte. Same surgical splice writes an edited `d`
  and a renamed `id`. `deleted` paths are dropped from render/export.
- **Source is editable + persistence is layered.** The SOURCE drawer re-parses
  edited SVG via `editor.load` (fail-safe: bad markup throws in `parseSvg`
  before anything mutates, so the doc is untouched and an error shows). The
  document + selection persist in localStorage; File System Access **handles**
  persist in **IndexedDB** (`workspace/handles.ts`) so save-back survives HMR
  and reload (permission re-checked, re-requested on the Save gesture).
- **Canvas gestures = an XState statechart** (`lib/canvas/machine.ts`, wrapped
  by `stores/canvas.svelte.ts`). States: idle · panning · dragging (+ a
  transient `gesture` that branches on whether the tool returned a DragSession).
  EditorCanvas sends DOWN/MOVE/UP/CANCEL; the machine owns *when* pan/move/up/
  cancel fire and holds the active session. Tools stay the behavior units; add a
  gesture state here, not another flag in the component. **Zoom/pan bypass the
  machine** (they're pure viewport, not edits): pinch — a trackpad pinch arrives
  as `ctrl+wheel`, and two fingers on a touchscreen are tracked directly in
  EditorCanvas — plus `⌘/ctrl+wheel` zoom; a plain wheel / two-finger scroll,
  space-drag, or middle-drag pans. A second pointer cancels any in-flight edit.
- **Live-edit then commit.** Mutations change the model continuously during a
  drag; the tool calls `editor.commit()` once at gesture end = one undo step. A
  plain click (no move) doesn't commit — selecting never dirties history.
- **Editing conveniences** live in the `+page` keyboard handler + the select
  tool: shift-drag axis-locks (nodes/handles/whole-shape), arrows nudge the
  selection (shift ×10), ⌘C/V/X/D copy/paste/cut/duplicate (an internal
  `#clipboard`; pastes are `added` paths offset +10,+10), Delete removes the
  selected node or path, Esc returns to the select tool.
- **Shared state = rune stores** (`frontend/src/lib/stores/*.svelte.ts`), read
  directly, never prop-drilled: `document` (doc + selection + mutations +
  history), `viewport`, `tool` (+ snap/grid settings), `workspace`, `interaction`
  (transient drag feedback), `settings` (theme + canvas backdrop).
- **Theming.** Light/dark/auto via `data-theme` on `<html>` (not a media query),
  so it's user-selectable in `SettingsDialog`. `halo.css` is light-first +
  `[data-theme='dark']`; the root `+layout` resolves the mode (auto → system,
  live) and `app.html` sets it pre-paint (no flash). `settings.canvasBg`
  (checker/light/dark) is the *artwork* preview surface — absolute, orthogonal
  to the UI theme. Accent stays the brand orange (a purple axis could drop in
  later, like scene's `data-accent`).

## Working on this repo

- **Toolchain:** Rust + `wasm-pack` (`cargo install wasm-pack`) + the
  `wasm32-unknown-unknown` target, alongside Node/yarn; optionally **binaryen**
  (`brew install binaryen`) for the `.wasm` size pass. `just build-core`
  (wasm-pack → `core/pkg`) runs first from `dev`/`build`/`install` so the
  frontend's `link:` dep resolves; `build` also runs `opt-core` (`wasm-opt -Oz`,
  skipped if binaryen is absent).
- Dev: `just dev` (or `cd frontend && yarn dev`) → http://localhost:5173.
- Validate: `just validate` (typecheck + lint + format). Tests: `just test`
  (`cargo test` + vitest); `just test-e2e` (Playwright browser smoke on a built app).
- **CI:** `ci.yaml` runs the core tests + validate + unit + e2e on every push/PR;
  `pages.yaml` builds + deploys the demo (size-optimizing the `.wasm` when binaryen
  installs). Both build the Rust core before the frontend.
- Yarn is the repo-vendored release (no corepack); recipes invoke it via node.
- **Pages deploy:** a project page lives under `/nib/`, so the Pages build sets
  `BASE_PATH=/nib` (→ `paths.base` in `svelte.config.js`); dev + any future
  backend build stay at the root (`BASE_PATH` unset). Manual asset links in
  `app.html` use `%sveltekit.assets%` so they resolve under the base. The
  workflow copies `index.html`→`404.html` for deep-link fallback; `.nojekyll`
  ships from `static/`.
- **Folder mode (open a folder, save back) is Chromium-only** (File System
  Access API). Fallbacks work everywhere: paste text / open single file / download.
- Hooks: `./install-hooks.sh` once (frontend lint+format pre-commit).

## Roadmap (post-Phase-0)

There is an approved roadmap to grow nib to a pro-tier vector editor on the
Rust/WASM core. **Phases A + B have landed.** Phase A (the core-first rewrite):
model, ops, geometry, parse/serialize, snap, undo in `nib-core`. Phase B (the
client-side pro pillars, all running on the core):

- **Landed:** stroke cap/join/dash + fill-rule, rect/line/polygon/star primitives,
  numeric-precision inspector; **unified layers = objects + groups** (Figma/
  Pixelmator model, one level — z-order/drag-reorder, show/hide, thumbnails, group/
  ungroup via `<g>`, active layer, right-click context menus); multi-select +
  marquee + align/distribute; rotate (about the box centre); **boolean ops**
  (union/subtract/intersect/exclude via the `i_overlay` kernel), **simplify** (RDP),
  **outline-stroke** (kurbo); smart guides; **gradients** (linear/radial, draggable
  stops, radial cx/cy/r); command palette (⌘K); plus workflow polish (New/Save-As,
  copy-style, source prettify + reveal, double-click node editing, friendly path
  names, content-aware fit + export viewBox, tight selection bounds).
- **B tails still open (nice-to-have):** offset-path (proper joins), rotate/skew
  about a *movable* pivot.
- **Phase C (next — additive, flag-gated):** rust-axum backend running the same core —
  op-log-over-WebSocket sync + an MCP tool surface. **The op vocabulary the editor
  already runs on IS the surface** (`moveNode` … `booleanOp` … `groupPaths`).
  Browser-only build stays fully functional.
- **Phase D (gated):** arbitrary *nested* groups — a full object tree on top of B's
  one-level named groups (needs stable-id addressing; imported paths currently group
  for membership/visibility/order but aren't `<g>`-wrapped on export).

The `added`/`attributes` model + op vocabulary + pluggable tools + grouped rail are
shaped to absorb these. If a feature crosses into an unbuilt area, check the
roadmap and raise scope before building.
