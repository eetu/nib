# nib — repo overview

A direct-manipulation SVG **path editor**: paste/open an LLM-generated SVG, drag
its anchor points and bezier handles, snap endpoints together to close loops,
save back. The LLM roughs out the shape; nib does the last-5% by-hand tuning
that prose can't. Sibling in eetu's homebrew family ([halo](../halo),
[ocular](../ocular), [scribe](../scribe)) — shares the halo-design system.

**Frontend-only** today: a fully client-side SvelteKit SPA. There is no backend
— nothing needs a server but serving the built files, and the app reads/writes
the user's own files via the File System Access API. A rust-axum serve-shell +
raspi deploy can drop in later without moving the frontend (the SPA already
builds to `dist/` with `fallback: index.html`, the family backend contract).

## Layout

```
frontend/         SvelteKit (Svelte 5 runes) + Vite SPA, adapter-static → dist/
.claude/skills/   nib-design skill (glyph, wordmark, layout, voice)
justfile          task runner (just dev / build / validate / test)
```

Per-area detail in `frontend/CLAUDE.md`.

## Conventions (the load-bearing invariants)

- **Model is pure TS, framework-free** (`frontend/src/lib/model`). Paths
  normalize to absolute cubic-bezier anchor nodes (M/L/H/V/C/S/Q/T/A all fold in
  via `svg-pathdata`); this is what makes the core testable and the editor
  growable.
- **Byte-for-byte preservation.** The original SVG source is kept; on export
  only *edited* paths get their `d` re-serialized (spliced in place). Everything
  else — other elements, attributes, unedited paths — is preserved verbatim.
  Arcs in an *edited* path convert to cubics (lossy); untouched paths never change.
- **Added (drawn) paths** carry `added: true` + their own `attributes` and have
  no source location. They render from the model (a Svelte-managed `<g class=
  "drawn">`, not the imperatively-imported artwork) and are *appended* before
  `</svg>` on export. Everything else treats them like any path (editable,
  snappable, undoable, persisted).
- **Two coordinate systems in the canvas.** Artwork is drawn in a scaled `<g>`
  (document units); the editing overlay is drawn in screen space so handles stay
  a constant pixel size at any zoom. `viewport.toScreen/toDoc` bridge them.
- **Tools are a pluggable seam** (`frontend/src/lib/tools`): each is
  `{ id, cursor, begin() → DragSession?, hover?() }`. Add a tool = add a module +
  registry entry; the rail groups them (`ToolRail`: select · create · nodes) so
  it stays scannable as they grow. `hover()` drives live aids (pen rubber-band,
  circle centre-snap). Tools: select/move, pen (draw new paths), circle (drag
  out a closed 4-node bezier), add-node, delete-node. Shapes are built as
  editable paths (`model/shapes.ts`), not native `<circle>`/`<rect>`.
- **Selection = node + path.** `selection` is the active node; `selectedPath` is
  an explicit path selection (PATHS row / path-body click). `selectedPathIndex`
  is the effective selected path: the selected node's path if any, else
  `selectedPath`. The STYLE panel targets it.
- **Object vs node mode (one tool, like Figma).** `objectSelected` = a whole
  path selected with *no* node (you clicked its body or a PATHS row). Only then
  does the Overlay draw the **transform box** — a dashed box (padded to clear
  the shape, `SELECT_PAD_PX`) + **8 resize handles** + an accent **centerline**
  (light casing + accent core so it reads on any stroke colour). Drag a corner
  (both axes) / edge (one axis) to scale about the opposite anchor; shift keeps
  aspect (`lib/tools/transform.ts` + the `transform` Hit kind + `select`'s
  scaleDrag). Clicking a **node** instead gives clean node editing (anchors +
  handles, no box). Hit-testing checks anchors *before* transform handles, and
  transform handles only when `objectSelected`, so a path's own nodes are never
  shadowed. Deleting the last node soft-deletes the now-empty path.
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
  gesture state here, not another flag in the component.
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
  (transient drag feedback).

## Working on this repo

- Dev: `just dev` (or `cd frontend && yarn dev`) → http://localhost:5173.
- Validate: `just validate` (typecheck + lint + format). Tests: `just test`.
- Yarn is the repo-vendored release (no corepack); recipes invoke it via node.
- **Folder mode (open a folder, save back) is Chromium-only** (File System
  Access API). Fallbacks work everywhere: paste text / open single file / download.
- Hooks: `./install-hooks.sh` once (frontend lint+format pre-commit).

## Out of scope (Phase 0 — deliberately not built)

Stroke cap/join/dash controls, a shapes flyout in the rail (when shape count
grows), other shape primitives (rect, polygon, line), boolean ops, rotate/skew
transforms (scale is built), multi-select, layers/groups, gradients, backend + hosting. The `added`/`attributes` model +
pluggable tools + grouped rail are shaped to absorb these. If a feature crosses
into those areas, raise it before building.
