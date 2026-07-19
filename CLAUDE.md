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
- **Native document model = the single source of truth (Phase C).** The engine's
  `SvgDocument` + structural `Tree` (nodes, geometry, styles, gradients, and every node's
  stable `uid`) is the authoritative model, serialized as **versioned JSON**
  (`Editor::toModel`/`loadModel`, `schemaVersion`). The backend persists + serves *this*
  (SQLite `projects.model`; the `svg` column is a cached export), and a client **loads the
  model** — it never re-parses SVG to sync. **SVG (and any future format) is import/export
  only.** This is what makes co-editing correct: identity is shared, so ops (structural
  uid-ops included) replay on every client. Standalone/browser-only still round-trips SVG
  fine; the model just rides in localStorage. *(JSON now; a binary serde encoding or a CRDT
  are documented future swaps behind the same types.)*
- **Dual identity — every node carries a `uid` AND a `name`.** `uid` = a stable, globally-
  unique (UUID) machine id, minted **once** at creation by whoever makes the node, **carried
  in the op** (create-ops carry it; the op funnels — frontend `stampCreateUid`, backend
  `ensure_create_uid` — stamp one if absent) and stored in the model, so no client ever
  re-invents it. `name` (the SVG `id`) = the human-facing label a co-author refers to ("the
  hand"): semantic, mutable, may collide. The MCP **`find`** tool resolves a name to candidate
  object(s) so the LLM disambiguates ("left or right?") instead of guessing. Rename changes
  `name`, never `uid`. This is the load-bearing invariant for human↔LLM co-authorship.
- **SVG export is canonical.** SVG is an *export* format now, not the store of record, so
  `to_svg` regenerates the whole document cleanly from the model (`serialize_canonical`):
  every element re-emitted from its tag+attrs, but **primitives kept** — a form-preserving
  `<rect>`/`<circle>`/… stays that primitive (via `refit`), only a freeform reshape falls to
  `<path>`. Byte-for-byte preservation is dropped as a contract; a faithful byte-preserving
  serializer (`serialize_via_tree`) is retained for round-trip tests + as a capability, but
  isn't the default. The native model — not any SVG form — is the identity substrate.
- **One representation: the document tree** (`core/src/model/tree.rs`, `Node`/`Tree`).
  Imported *and* drawn content are nodes in the same tree — imported nodes parse from
  source (verbatim spans, byte-for-byte re-emit); **drawn (added) paths** are `<path>`
  nodes created by `add_drawn` (a fresh uid, appended to the root = top of z-order),
  carrying `added: true` + their whole style in `attributes`. Every editable shape
  projects to a flat `PathElement` (by `uid`) that tools/geometry/undo run on; edits
  reconcile back onto its node by uid on serialize. There is **no separate drawn
  zone** — the canvas renders one `g.artwork` from `editor.renderTree()`, the LAYERS
  panel renders the same tree, and export walks the same tree.
- **Groups = `<g>` nodes at any nesting depth.** The Inspector **LAYERS** panel *is*
  the object tree: every path/shape is a row, every `<g>` a collapsible group (nested,
  reversed so top-of-stack shows first), **z-order = document order**. **Group**
  (`GroupNodes {uids,uid,name}`) wraps sibling nodes in a new `<g id="name">`;
  **Ungroup** (`UngroupNode`) splices its children back into the parent; **reorder**
  (`ReorderNode {uid,forward}`, bring-forward/send-backward in the row context menu)
  swaps a node with its adjacent element sibling; **drag a row** (`MoveTreeNode {uid,refUid,
  position}`) reorders arbitrarily + moves nodes in/out of groups (before/after a sibling, or inside
  a group). **Show/hide** is per-node
  (`SetNodeHidden` → `Node.hidden`, a group + its subtree) or per-path (`SetPathHidden`
  → `PathElement.hidden`); both export `display="none"`. **Export is byte-for-byte
  until something is edited/grouped/reordered/hidden/drawn** (grouping/reordering is an
  active re-serialization of structure). *Caveat:* `GroupNodes` needs the selection to
  share one parent (top-level selections always do); cross-level grouping is a no-op.
- **Live (non-destructive) booleans** are a `<g>` node with `Node.boolean_op` set
  (union/subtract/intersect/exclude). Its element children stay editable **operands**;
  the doc renders + exports the *computed* boolean of them (subject = the first child
  that fills), recomputed live as operands change. This is a **parametric concept SVG
  can't express**, so it lives in nib's model (the tree is persisted in localStorage) —
  on **serialize the `<g>` bakes to one `<path/>`** (source = export = valid SVG
  everywhere), and re-parsing edited source flattens it to a plain group (lossy but
  graceful). The computed geometry is *not stored on the doc* — the core recomputes it
  in `state()` from the tree over live `doc.paths` and hands the UI
  `EditorState.booleanResults` (`{uid, subpaths, attributes, operandUids}` per group,
  keyed by the group node's uid). Ops: `SetNodeBoolean {uid,op?}` (set/flip/flatten);
  the facade's `makeBooleanGroup` does `GroupNodes` + `SetNodeBoolean` as one undo step.
  Frontend: `EditorCanvas`'s `renderNode` paints a `<g booleanOp>` node's computed
  result (from `booleanResults`, keyed by uid) instead of recursing into the operands;
  operands stay **hit-testable via model geometry** (click/drag/node-edit → live re-cut)
  and show as faint dashed outlines in the Overlay (by `operandUids`) when the group is
  active. UI: a "live (non-destructive)" toggle on the multi-select boolean buttons +
  palette + a group-header badge/context-menu. Distinct from the **destructive**
  `BooleanOp` (bakes + deletes inputs immediately).
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
- **Selection = node + path (+ element).** `selection` is the active node;
  `selectedPath` is an explicit path selection (PATHS row / path-body click).
  `selectedPathIndex` is the effective selected path: the selected node's path if
  any, else `selectedPath`. The STYLE panel targets it. `selectedElementUid`
  (orthogonal — each clears the other) selects a **non-shape element** (text/image/
  use) by its tree `uid`; the Inspector's element section edits it. **Group selection
  (Figma-style):** clicking a shape inside a `<g>` selects the whole outermost group
  (`selectGroup` → its member path indices as a multi-selection + `selectedGroupUid` to
  mark it a group, so clicking a member doesn't reduce it); double-click drills in to
  node-edit that shape.
- **Non-shape elements (text/image/use) are transformable objects (E4).** They
  aren't editable paths (no anchor geometry), so they carry no `PathElement`;
  instead the canvas selects them by `data-uid` on the rendered DOM (when the model
  hit-test misses), measures their box from the rendered DOM (`getBoundingClientRect`
  — the model can't know font metrics), draws the shared transform box, and
  move/resize/rotate compose an SVG `transform` matrix on the node (a plain move with
  no existing transform edits `x`/`y` for clean markup). Ops: `SetNodeAttr {uid,key,
  value?}` (any attr — x/y/width/height/transform/fill/font-size) + `SetNodeText
  {uid,text}` (content). The Inspector element section edits authored attrs + text.
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
Rust/WASM core. **Phases A, B, and E have landed — the editor is feature-complete; the remaining
editor work is finalization (real-SVG corpus coverage, robustness, large-doc perf, UX polish → 1.0).
Phase C (backend/MCP/sync) is the parallel/after track.** Phase A (the core-first rewrite):
model, ops, geometry, parse/serialize, snap, undo in `nib-core`. Phase B (the
client-side pro pillars, all running on the core):

- **Landed:** stroke cap/join/dash + fill-rule, rect/line/polygon/star primitives,
  numeric-precision inspector; **unified object tree** (imported + drawn content are
  one tree of nodes — nested `<g>` groups at any depth, z-order = document order,
  show/hide, thumbnails, group/ungroup/reorder, right-click context menus; the flat
  `doc.layers` model is retired — see the "One representation" convention); multi-select +
  marquee + align/distribute; **rotate** (box centre) + **skew** (numeric);
  **path craft** — boolean ops (union/subtract/intersect/exclude via `i_overlay`,
  both **destructive** *and* **live/non-destructive** boolean groups),
  **compound paths** (combine/release), **simplify** (RDP), **outline-stroke** +
  **offset-path** (kurbo stroke ⊕ i_overlay);
  smart guides; **gradients** (linear/radial, draggable stops, radial cx/cy/r);
  command palette (⌘K); plus workflow polish (New/Save-As, copy-style, source
  prettify + reveal, double-click node editing, friendly path names, content-aware
  fit + export viewBox, tight selection bounds).
- **1.0 tools + fidelity — landed this cycle (all core-op-first, so MCP + sync get them free):**
  first-class **rotation** (`RotatePath`; numeric + 90° buttons + MCP) and **flip H/V** (`FlipPath`,
  ⇧H/⇧V); a **drop-shadow** effect (`SetDropShadow` → an `feDropShadow` filter def); **rounded-rect**
  corner radius (`ShapeSpec.rx/ry` + an interactive rect-tool radius); **bring-to-front / send-to-back**
  (`ReorderNodeExtreme`, ⌘]/⌘[ + ⌘⇧]/⌘⇧[, MCP `reorder`); **lock/unlock** (`SetPathLocked` — editor-only,
  hit-test-skipped, never exported); **select-all** (⌘A); a **text creation tool** (`AddText` + MCP
  `add_text`) and an **eyedropper** (`sampleFillAt`); **⌘/Ctrl snap-bypass** + **snap-to-grid on shape
  drag**; **reusable components** (a `<g>` in `<defs>` projects as editable shapes, `<use>` instances,
  stamp, edit-once-propagates, plus **detach**-to-bake and **delete**-cascade) with a full MCP surface
  (`create_component`/`stamp`/`list_components`/`group_named`); **export-fidelity fixes** (canonical
  export was dropping the root `xmlns` and mangling namespaced attribute prefixes; `<use>` now emits
  `xlink:href` back-compat). Header/rail **UX pass** (cluster dividers, centred document title,
  consolidated tool groups).
- **Open issues → 1.0 (finalization — verification + polish, not new capability):**
  1. **Export fidelity on a real-SVG corpus — automated half LANDED.** `core/tests/fidelity.rs`
     rasterizes **source vs. canonical export** with resvg (a dev-dep, so it never touches the WASM
     build) and pixel-diffs the whole corpus — the render-grade gate the byte-preserving
     `roundtrip.rs` can't be (canonical *regenerates* markup). It immediately caught a real bug:
     canonical regenerate-all dropped the **namespace prefix on element open tags** (`<format>…
     </dc:format>` → unparseable XML), now fixed in `tree.rs` `build` by reconstructing `prefix:local`
     for element names like it already did for attrs. Corpus grew with Figma clip-path/rounded-rect,
     an SVGO one-liner with group-inherited fill, and radial/stop-opacity gradients. *Remaining
     (manual):* open a handful of exports in a real design app (Pixelmator Pro / Figma / Inkscape) —
     the resvg gate proves render-equivalence, the manual pass confirms strict third-party importers.
  2. **Pixel-verify the new tools** end-to-end (rotate/flip/rounded-rect/drop-shadow/text/eyedropper)
     via `render_document` + a manual pass (text needs system fonts to raster).
  3. **Large-document performance** — profile project/reconcile/serialize + canvas render at
     hundreds–thousands of nodes; the invariants are linear but unmeasured at scale.
  4. **UI follow-ups — LANDED:** the duplicated grid toggle now lives *only* in the header snap
     popover (the rail button is gone); the **basic/advanced** UI level gets a **first-run chooser**
     (`WelcomeDialog`, shown once when `settings.uiLevelChosen` is false — persisting the pick retires
     it, then it's Settings-only). e2e seed `nib:uiLevel` in a `beforeEach` so tests boot as returning
     users; a dedicated first-run test covers the chooser.
  5. **Deferred (needs a dedicated rotate tool):** rotate/skew about a *freely-movable* pivot — a
     centre pivot handle conflicts with the unified select tool's drag-to-move + double-click-to-node-
     edit. Then **freeze the editor UI (1.0 RC).**
- **Editor track = A → B → E → finalize; Phase C rides alongside.** Phase E is the
  *editor's capstone* — once it lands the editor is feature-complete and the remaining work
  is **finalization** (coverage/fidelity on a real-SVG corpus, robustness + large-doc perf,
  UX polish, ship a 1.0), not new capability. **Phase C is not part of "the editor"** — it's
  the co-editing/persistence infra wrapping the *same* core, a parallel/after track; the
  browser-only editor stays fully functional without it.
- **Phase C (additive, flag-gated): the backend co-editing track — mostly LANDED.** A
  rust-axum backend (`backend/`) links the **same `nib-core` natively** and now persists
  **projects** in **SQLite** (sqlx), owned by **token-authed users** — a real multiuser scaffold
  (a seeded `developer` user + per-user bearer token; no login yet). **The op vocabulary the
  editor already runs on IS the surface** (`moveNode` … `booleanOp` … `groupNodes`). What's live:
  - **Persistence + auth** (`db.rs`, `auth.rs`): `migrations/` (users, projects). The store of
    record is the **native model JSON** (`projects.model`, migration `0002`; the `projects.svg`
    column is a cached export). `Authorization: Bearer <token>` (an `AuthUser` extractor + an MCP
    helper). REST: `/api/me`, `/api/projects` (list/create), `/api/projects/{id}` (get → model+svg /
    put imports svg → model) — token-authed + ownership-scoped, validated through the parser.
  - **Sessions** (`session.rs`): one authoritative in-memory `Editor` per open project (keyed by
    id, shared registry), hydrated **from the model** (legacy svg-only rows import once). Every edit
    funnels through `apply_ops` → `ensure_create_uid` (stamp a uid on create-ops that lack one) →
    mutate → **broadcast ops** → persist model. All clients share the model (identical `uid`s), so
    ops — structural uid-ops included — replay correctly; no snapshot resync.
  - **MCP** (`mcp.rs`, `rmcp` 0.5) nested at **`/mcp`** (Streamable-HTTP): token-authed +
    project-scoped. Tools: `list_projects`/`create_project`/`open_project`, `get_document`
    (a **cheap text outline** — one line per path: `#index`, name, bounds, fill/stroke),
    **`find`** (resolve a co-author's *name* — "the hand" — to candidate objects with #index +
    bounds so the LLM disambiguates "left or right?" instead of guessing), `get_svg`,
    **`render_document`** (rasterize to a PNG via `resvg` + return it as an **image** so the LLM can
    *see*/verify its work; opt-in `width` cost knob), **`apply_op`** (full op vocabulary), + ergonomic
    wrappers `add_shape` (optional `name`)/`set_style`/`boolean_op`/**`group`** (indices→`GroupNodes`
    by tree uid)/**`rename`**. The surface is **shaped to coach the model** (mirrors the sibling
    `../maquette`): a workflow playbook in the server `instructions`, per-tool descriptions that say
    when *not* to spend an expensive call, and mutations that return a **one-line ack** (never the
    whole doc) — so the LLM names + groups shapes into a labeled hierarchy and spends few tokens per
    step. Structural ops (group/boolean/reorder) renumber `#index`, so the acks say "call get_document".
  - **C2 live sync** (`sync.rs`): `GET /ws/projects/{id}?token=…` — the browser + the LLM edit the
    **same project live**; MCP `apply_op` broadcasts to the WS, and WS ops broadcast back
    (echo-guarded by `clientId`).
  - **Frontend connected mode — LANDED (flagged):** build-flagged (`PUBLIC_NIB_BACKEND`) so the
    **standalone / GitHub-Pages build ships zero backend code** and stays a pure local file editor;
    when on, a projects list + token-in-Settings + `ProjectSync` (WS) + `DocumentStore.applyRemote`
    make the co-editing visible in the browser. Plan: `~/.claude/plans/happy-crunching-blum.md`.
    **Phase C is functionally complete as a co-editing scaffold; a real login/multi-tenant story +
    hardening (rate limits, conflict UX) remain before it's production-grade.**
- **Phase D — LANDED (folded into E3):** arbitrary *nested* groups are the object tree
  itself — `GroupNodes`/`UngroupNode`/`ReorderNode`/`SetNodeHidden` on stable-id (`uid`)
  addressing; drawn + imported content unified into one tree, `<g>`-wrapped on export.
- **Phase E (the big model shift — E1 flip + E2 LANDED):** grew from **paths-only** (rest
  preserved as an opaque source string) toward a **full SVG element tree** (`core/src/model/
  tree.rs`), so **save re-emits the tree** ("import → native → export" cornerstone). Key
  insight kept it safe: **per-node original text** (unedited node re-emits verbatim →
  byte-preservation generalizes from paths to *all* elements; only edited nodes change).
  **What's live:** the `Editor` holds a parsed `Tree` as a **constant serialization base**;
  `doc.paths` stays the mutable working model (**ops + undo unchanged**), seeded via
  `Tree::project_paths` so **imported primitives (`<rect>`/`<circle>`/`<ellipse>`/`<line>`/
  `<polygon>`/`<polyline>`) are editable paths** (each carries a stable `uid`); `to_svg` =
  `serialize_via_tree` (reconcile flat edits by uid onto a tree clone — edited primitive →
  `<path>`, deleted dropped, siblings verbatim, drawn paths + baked booleans emitted from
  their tree nodes — then inject defs + grow viewBox). Edited primitives **re-fit** on export — a form-preserving move/resize stays
  `<rect>`/`<circle>`/… with updated attrs, only a freeform reshape falls to `<path>`
  (`tree::refit`). **Rendering is fully declarative (#30 landed):** `EditorCanvas` draws the
  whole document from `editor.renderTree()` — editable shapes as `<path>` from the model in
  true z-order, opaque elements verbatim via `<svelte:element>`; the imperative import is
  retired. Fidelity gate: `core/tests/roundtrip.rs` corpus. **E3 real nested groups LANDED**
  (the unified object tree: drawn + imported content, nested `<g>` groups, live booleans, all
  one tree — subsumes D; reorder/group/ungroup go through the tree so they reflect on export).
  **E4 transformable text/image/use LANDED:** non-shape elements are first-class objects — click-
  select on canvas (DOM `data-uid` when the model hit misses) + a transform box (measured from the
  rendered DOM bbox) with move/resize/rotate composing an SVG `transform` matrix (a plain move with
  no existing transform edits x/y for clean markup); the Inspector's element section edits authored
  attrs (x/y/w/h/font-size/fill) + text content via `SetNodeAttr`/`SetNodeText`. **E5 (defs) in
  progress:** `<defs>` content is inert — clip/mask/filter/gradient defs render + round-trip
  byte-for-byte, but their inner shapes no longer project as phantom editable paths (`DEF_CONTAINERS`
  skip in `collect_paths`); imported source gradients that fit the model (objectBoundingBox) are
  **editable in place** — the first edit adopts them into `doc.gradients` keeping their id, and
  `serialize_via_tree` drops the source def (`Tree::remove_gradient_defs`) so it defines once
  (byte-for-byte until adoption); ones that don't fit (userSpaceOnUse/gradientTransform/…) stay
  read-only. *(The default save is now the canonical export — see the "SVG export is canonical"
  cornerstone; the separate paths-only "export normalized copy" was retired as redundant once
  canonical became the default.)* **E5 — and the editor track (A→B→E) — is complete; what remains is finalization**
  (coverage/fidelity on a real-SVG corpus, robustness + large-doc perf, UX polish, ship 1.0).
  Full plan: `~/.claude/plans/nib-full-svg-dom.md`.
  Paired UX **(landed early, ahead of E):** a persisted **basic/advanced** UI preference
  (`settings.uiLevel`, default **advanced**) — *basic* is the opt-in that declutters to
  touch-up tools (select/node-edit/solid-style/save; hides the shapes rail group, arrange,
  path craft, booleans, gradients, grouping, skew — and makes their shortcuts inert);
  *advanced* is the full pro surface. Engine + LLM/MCP op surface stay full regardless; the
  toggle only gates chrome. `ToolGroup.advanced` + `ADVANCED_TOOL_IDS` gate the rail +
  shortcuts; `settings.uiLevel === "advanced"` gates Inspector sections + PaintInput
  gradients; `SettingsDialog` switches it.

The `added`/`attributes` model + op vocabulary + pluggable tools + grouped rail are
shaped to absorb these. If a feature crosses into an unbuilt area, check the
roadmap and raise scope before building.
