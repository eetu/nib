# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Three tracks since 0.1.0: the editor grew the last of its 1.0 tools, the MCP surface grew from
"place shapes" into something you can actually draw with, and the interface was rearranged to the
shape the whole family is meant to share. The MCP surface is the one still moving — see the note at
the end.

### Added

- **Text converts to outlines.** A label carries no anchor geometry, so it can't be node-edited,
  boolean'd or offset, and it renders wrong wherever the font is missing. `TextToPath` replaces the
  `<text>` **in place, keeping its uid**, with a `<path>` of its shaped glyphs. Shaping is in the
  core (rustybuzz + ttf-parser — HarfBuzz-grade kerning, ligatures, RTL), but **font bytes are a
  host resource**: the browser reads them from the Local Font Access API or a picked file, cached
  per family+weight+style in IndexedDB; the backend resolves them with fontdb. Surfaces: the
  Inspector, ⌘K (one label or all), and MCP `outline_text`.
  - **`.woff2` decodes in-core**, because a font *download* is a `.woff2` — it's what someone
    actually has on disk when they go looking for a face.
  - **A tspan label converts as the lines it is**, so design-tool exports (Figma and Illustrator
    write multi-line text as tspans) are outlinable at all.
  - **A substituted face is announced.** The outlines are right either way, but they're no longer
    the letterforms the document named, and silence there reads as nib having mangled the text.
  - **Fonts ship in the container** (~7.5MB curated from Liberation + DejaVu). A `scratch` runtime
    otherwise has none, which would leave server-side `outline_text` with nothing to shape and
    `render_document` drawing no words — working locally, broken deployed. `NIB_FONT_DIR` adds a
    mounted directory for brand faces without a rebuild.
- **A label's typeface is editable** — family (free text with installed-font suggestions where the
  browser will list them), weight, and slant. A family is a *name in the document*, not a choice
  from a list: a font this machine lacks is still right for a file that opens elsewhere.
- **Rotate about a pivot you place** — its own tool (`e`). The select tool's box turns about its
  centre, which covers most rotation; what it can't do is swing a shape *around something else*, an
  arm about a shoulder. Click places the pivot, drag turns about it; numeric rotate and skew honour
  it too.
- **A selection box that stays turned**, Pixelmator-style, with handles on the object's own axes —
  so dragging the east handle of a box turned 30° widens the shape along its own edge instead of
  stretching it across the document's x. Shapes needed the tilt *stored* (`PathElement::box_angle`,
  a nib annotation like `locked` — it persists and syncs but is never written to SVG); a
  text/image/use element already stored it as a node `transform`, so its box is measured from its
  own untransformed bbox through its screen matrix instead. Both draw and hit-test through one
  `BoxFrame` of screen-space points, so the overlay and the hit-test can't drift apart.
- **The MCP surface can draw**, not just assemble shapes:
  - **`draw_path {d}`** takes SVG path data straight into the core's parser — the same road an
    import takes — so anything curved is expressible and lands as ordinary editable anchors. The
    vocabulary was previously ellipse/rect/line/polygon/star, which made every organic form an
    ellipse.
  - **Bulk targeting.** `rotate`/`scale`/`move`/`flip`/`mirror`/`set_style`/`set_gradient`/
    `duplicate`/`reorder` each take `index`, `indices`, **or a `name`** — and a name that resolves
    to a `<g>` acts on every shape inside it. Tilting a nineteen-shape scene was nineteen identical
    calls; it's one.
  - **`scale`** (a real `ScalePath` core op, so the browser gets numeric resize too) and **`move`**.
    There was previously no way to resize a shape through MCP at all, not even via `apply_op`.
  - **`mirror`** — copy and flip about a line in one call, renaming left↔right. Building a
    symmetric half by authoring mirrored coordinates is what it replaces.
  - **`measure`** — a shape's *own* box: width, height, tilt, centre, and four corners. It prefers
    `box_angle` but falls back to reading the tilt out of the geometry (four corners with square
    angles IS a rectangle), since anything rotated before nib recorded angles reports nothing —
    which is exactly the case that keeps coming up when placing work inside an imported frame.
  - **`apply_ops`** — a batch, so a drawing costs a handful of round trips instead of forty-five.
  - **`undo`/`redo`** — they can't broadcast as ops (a peer replays ops against its own copy, and
    "undo" is a statement about *this* document's history), so they ride the `reload` channel a
    whole-document import already uses. The history is the document's, **shared with the human**,
    and the tool descriptions say so.
  - **`duplicate`**, **`set_gradient`** (a real `<defs>` entry with the browser's own angle
    parameterisation, so the human can pick up its stops), and **cropped rendering**
    (`render_document { around }` or an explicit region) — rendered through a translate into a
    region-sized pixmap, never by scaling the whole document and cutting it up.
  - `get_document` reports a **rotated shape's own size and angle** beside its document-axis box —
    a 57×74 frame tilted 26° reads 84×92 on the page, which is nobody's idea of its size — and its
    bounds now include control handles, or an arc drawn from two anchors reports height 0.

### Changed

- **The window is laid out by role now** — navigate · surface · subject · tools, left to right —
  which is the arrangement the family skill describes and nib is the first to implement. Layers,
  projects and files became **tabs** in a left-hand navigator (they're all lists you go to, pick
  from and leave; stacked they competed for one column, where thirty files could push the object
  tree off a laptop). The Inspector kept the right, answering "what am I working on?". And the tool
  rail moved beside it: a rail on the far side of the canvas from the options it produces is two
  unrelated strips, and adjacency is the point of pairing them. Each region folds, and the fold is
  a global pref — chrome layout doesn't belong to a document.
- **A paint is a switch plus a kind**, not a row of four chips. `none` was competing with
  solid/linear/radial in one segmented control, which is why "no fill" ended up with two separate
  controls a week apart — `none` isn't a *kind* of paint, so it kept wanting a second home — and
  why the panel showed eight chips before you reached a colour. A switch owns whether the paint
  exists, a short list owns what kind, and only the chosen kind's controls are on screen; switching
  a paint off collapses its block. `currentColor` became a kind, which retired the keyword picker
  added days earlier to patch the first symptom.
- **A new shape is drawn in a real colour** (`#111111`), not `currentColor`. Following the UI theme
  was clever and wrong three ways: nib produces a FILE, and `stroke="currentColor"` renders a
  different colour in every context it lands in; the artwork is judged against `canvasBg`, which is
  orthogonal to the chrome's theme; and every shape carrying the same deferred keyword made the
  eyedropper return the same grey whatever you pointed at.
- **One control per meaning in the interface**, aligning with the halo-interaction rules: a single
  root context menu (the canvas has one now, with node verbs on an anchor), the browser's own menu
  suppressed app-wide except in text fields, one Modal primitive behind every dialog, and **Revert**
  — a reload has to be undoable.
- **The paint field's `none` appears once.** The fill block had a `—` chip in the mode row *and* a
  `—` button under it; the button is now a picker for the values a swatch can't express, so
  `currentColor` is reachable at all.
- **A tilt tool** (`k`, advanced) — drag an edge of the selection box and the opposite edge stays
  pinned, so the shape leans. One drag carries both halves of the gesture: the motion along the
  edge shears, the motion across it foreshortens, which is what makes it read as a face tipping
  away rather than as two separate operations. This is *flat* perspective and says so — parallel
  edges stay parallel, which is what isometric illustration is built from. True vanishing-point
  perspective is a projective map, and a projective map sends a cubic bezier to a **rational**
  cubic that SVG cannot express, so it would have to subdivide and approximate; an affine map
  sends a cubic to a cubic, so moving the anchors with their handles *is* the transform — exact
  and reversible. That one is deliberately post-1.0.
  - Only the four EDGE handles are live, and the corners and rotate knob aren't drawn. A corner
    drag can't specify an affine map (pinning the opposite corner leaves two corners free — one
    equation, two unknowns), and a handle that says "pull me" and then does nothing is worse than
    an absent one.
  - Anchors are hidden and un-hit-tested while a transform tool is active (the rotate tool too,
    now). Not tidiness: a circle's four anchors sit exactly on its box's edge-midpoint handles,
    and anchors win the hit-test, so without this the tool could not grab an ellipse at all.
- **`skew` and `transform` on the MCP surface**, and a real `AffinePath` op under both. Skew had
  no op: the interactive and numeric versions computed geometry in the frontend and wrote it as a
  wholesale `setSubpaths`, so the intent never reached the core — which is why the LLM could
  rotate, scale, move and flip a shape but not shear one, and why a peer replaying the edit
  received a pile of coordinates instead of a transform. The numeric skew now routes through the
  op like rotate already did.
- **The canvas has a size you can set** (`SetViewBox`, crops or pads). nib previously had no way
  to change a canvas at all. It comes with a wrinkle worth knowing: export normally *grows* the
  viewBox to cover content drawn outside it, so a shape past the edge isn't clipped when the file
  opens elsewhere — right for content that escaped, wrong for a size someone chose, since cropping
  is the entire point. So a chosen size turns that net off and exports verbatim.
- **With nothing selected, the right panel shows the DOCUMENT** — canvas origin and size, the shape
  count, fit-to-artwork. Deliberately not an auto-selection: selecting is a statement the user
  makes, and a shape selected on their behalf is one Delete away from being acted on unnoticed.
- **The eyedropper draws a loupe** while armed — the colour it would take and the *named shape* it
  comes from. Deliberately not a pixel magnifier: nib samples the model, so magnified pixels would
  show antialiased edge colours it can never return, lying harder the closer you looked.
- **The eyedropper moved out of the tool rail and into the paint rows**, one per paint. It sat
  between pen and text, which answer "what am I drawing?"; an eyedropper answers "what colour?",
  and the field it fills was across the window — while the STYLE header's copy-style button drew
  the same pipette glyph a few pixels away. One per paint also settles which paint a sample lands
  in: the button beside `stroke` takes a stroke colour, and lights while it's armed. The `i`
  shortcut still works and still means fill. Making room meant dropping the empty 50px label column
  the value row opened with — 56px of a 232px panel — so the hex field is wider than it was before.
- **The projects panel closes to a rail** (it keeps the sync status visible, and remembers).
- The container image is built only on PRs that touch it, as the workflow comment already claimed.

### Fixed

- **Concurrent co-editing hit `database is locked`.** SQLite's default rollback journal excludes
  readers for the duration of a write, so the human's `PUT`, the LLM's op and a page load landing
  together failed under ordinary use rather than under load. WAL lets readers run alongside the
  writer, and a 5s `busy_timeout` makes the one remaining case — two writers — wait its turn.
- **One panic made a project permanently unopenable.** A `std::sync::Mutex` stays poisoned for the
  life of the process, so a single `lock().unwrap()` turned one bad op into that project being
  dead for every user until a restart. Locks now recover the guard: what's behind it is a document
  model, where the worst a half-finished mutation leaves is an edit the next op overwrites and
  undo can take back. A `CatchPanicLayer` turns the panic itself into a 500 with a log line.
- **The REST surface had no timeout and a 2MB body cap** — a real illustration `PUT` was rejected
  with a bare 413. Now 16MB and 30s, applied to `/api` only: the WebSocket and MCP's
  Streamable-HTTP stream are long-lived by design, and a global timeout would cut the co-editing
  session itself.
- **A text layer couldn't be deleted at all.** Every delete in the app addressed a `PathElement`
  by index — but a `<text>`, `<image>` or `<use>` has no editable geometry, so it projects no path
  and has no index; neither does a `<g>`. The layer row for one had no context menu, the canvas
  menu offered hide but not delete, and the Delete key looked only at node and path selections. A
  new `DeleteTreeNode {uid}` op addresses the tree instead, so it reaches every node kind and
  takes a group's whole subtree with it; the row menu, the canvas menu, ⌫ and the palette all
  route through it. Callers reselect **by uid**, since removing a subtree re-projects the paths
  view and every index after it shifts. MCP gets a **`delete`** tool over the same op — the only
  way it could remove a label, image or `<use>` either, for the same reason; a label with no id
  answers to its own words, the way `outline_text` already addresses one.
- **A reload left connected mode attached to nothing.** The canvas came back — the document
  rehydrates from localStorage — but the project didn't, so every edit went nowhere and the only
  cue was an unhighlighted row in a panel you might never open. The reattach lived inside that
  panel, which is why it was silent: it only ran if you happened to be on the projects tab. It now
  boots with the header, and the header says which of the three states you're in ("synced" ·
  "local copy" · the connection being down), because if silence meant "fine" it would also mean
  "this didn't render". On restore the **server's copy wins**: it's what every other client and
  the LLM are working from, and a local copy that drifted while detached is exactly what must not
  be pushed over it.
- **`group_named` rejected the names it had just handed out.** A name lives in one of two places
  depending only on where the shape came from — an `id` attribute on the tree node (imported
  shapes, and every `<g>`), or on the `PathElement`, which is where a shape drawn through MCP
  keeps its name. The structural lookup read only the first, so `set_style` accepted "sky3" while
  `group_named` insisted no such shape existed, and drawing a scene and then grouping it by name
  was impossible for no reason the caller could see. One resolver now answers for both, and
  `reorder`'s copy of the fallback collapsed into it.
- **An element drag ran away from the cursor, accelerating** — in Firefox, and subtly everywhere.
  `consolidate().matrix` is a *live* view in some engines, so every pointermove composed onto the
  previous frame's result instead of the gesture's start.
- **Panning collapsed to ~4fps in Safari** on a zoomed-in document with a drop shadow. WebKit sizes
  a filter's surface in *device* space, so a shadow at 400× zoom becomes a several-hundred-megapixel
  blur re-rasterized every frame (Chromium clips it to the viewport). Filters now go quiet while the
  view is moving and the accurate frame paints the moment the gesture ends.
- **"Convert to outlines" did nothing in Safari** — the file picker has to open in the gesture's own
  task, so the font is now warmed on selection rather than fetched after an await.
- **A shared dev token shipped as a real credential**, and a pre-OIDC dev database couldn't boot.
- **The eyedropper answered for the whole scene.** `pointInPath` is a winding test on geometry
  that knows nothing about paint, so a `fill="none"` shape "contained" every point inside its
  outline. A picture frame drawn as an unfilled rect and brought to the front therefore answered
  every click — falling past its none-fill and returning its stroke, the same colour everywhere. A
  shape now answers only where it *paints*: its fill inside, its stroke within the stroke's width.
- **The eyedropper returned references, not colours** — `currentColor` and `url(#gradient)` came
  back verbatim. A sampled paint now resolves: to what it actually renders as, or to a gradient's
  first stop.
- **Arming the eyedropper ended the path you were drawing.** Switching tools runs the outgoing
  tool's cleanup, and the pen's cleanup is "finish the path" — so reaching for a colour mid-stroke
  ended the line being coloured and dropped you on the select tool. A momentary tool is now
  *borrowed*: the host is suspended and handed back, the UI keeps describing the host, and Escape
  releases the interlude above the host's own rung.
- **An MCP group's uid was a per-process counter.** Reopening a project and grouping again minted
  `grp-1` a second time, colliding with the group the previous session persisted — two nodes with
  one uid, after which every uid-addressed op found whichever came first in the tree. uids are the
  identity substrate that makes ops replay across clients, so this was wrong for sync and structural
  edits, not just for the name lookup that exposed it. Gradient and filter ids had the same bug one
  level down.
- **`flip`'s pivot needed both `cx` and `cy`** or it silently fell back to the shape's own centre,
  so `flip { cx }` mirrored in place and reported success.
- **A name resolved through tree `id` attributes only**, but a shape you just drew keeps its name on
  the path and has no tree id — so `drew #0 "wave"` was followed by `no shape named "wave"`.
- A `line` could only be its bounding box's ↘ diagonal, so "/" was unreachable; it now takes
  endpoints.
- Editing a label's words is a double-click on the canvas, and elements nested under a transformed
  group no longer fly off when dragged.

### Still ahead of 1.0

The editor's feature work is done; the MCP surface is not settled. Every tool added above came out
of actually drawing something through it and noticing where it fought back, and that loop hasn't
converged yet — expect more pair-drawing sessions and more fine-tuning before a 1.0 tag.

## [0.1.0] - 2026-08-02

The first self-hostable release: nib runs on the Pi as a container, signs users in with Kanidm,
and exposes its editing engine to an LLM over MCP.

### Added

- **Real users via OIDC.** nib is its own OIDC client (`openidconnect` + PKCE, the sibling apps'
  `oidc.rs` shape). `/auth/login` → issuer → `/auth/callback` sets a signed `nib_session` cookie;
  identity is the issuer's `sub`, so a rename or a changed address follows the same account.
  Discovery is lazy and self-healing, which is what makes the two-deploy Kanidm bootstrap safe.
- **A personal, rotatable bearer token per user**, minted at first login. Settings shows it with
  copy + rotate; it's the credential an MCP client presents. Reading or rotating it requires the
  browser session — a leaked token can't read itself back or mint its replacement.
- **`/mcp` bypasses SSO** and authenticates with the bearer alone.
- **Importing an SVG into an open project.** Drop a file on the canvas (or Open / Paste / edit the
  source) while a project is open and it now goes *into* that project — pushed with
  `PUT /api/projects/{id}`, then re-loaded from the server so every client and the LLM share the
  same node identity. Starting a New document detaches from the project instead.
- **Projects can be renamed and deleted** from the projects panel — double-click a row to rename
  in place, right-click for rename/delete (mirroring the Inspector's LAYERS rows). Backed by
  `PATCH`/`DELETE /api/projects/{id}`, both ownership-scoped in SQL; deleting drops the project's
  in-memory session so nothing keeps serving a document whose row is gone. Also on MCP as
  `rename_project` / `delete_project` — the latter requires the project's current name alongside
  its id, so a hallucinated or stale id fails loudly instead of destroying the wrong document.
- **Container packaging**: a 5-stage Dockerfile (the family's `xx` cross-compile → `scratch`, plus
  a wasm-pack stage for the core the SPA links) and an arm64 image published to
  `ghcr.io/eetu/nib`.
- Backend `clippy`/`rustfmt`/test job in CI — the backend was never compiled there before.

### Fixed

- **A document replaced while a project was open silently corrupted it.** Dropping or opening an
  SVG swapped the canvas but left the project untouched, and sync stayed connected — so later
  edits were ops built against the imported document but applied to the server's old one. The
  project ended up holding a degenerate path matching neither. Whole-document swaps are now an
  explicit event (`importDocument`) that connected mode either pushes to the project or detaches
  on; and `PUT /api/projects/{id}` updates the **live session**, not just the row, so an attached
  browser or MCP client sees the import instead of editing on over the top of it.
- **Cross-tenant session bypass.** `session::open` checked project ownership only on the cold
  path, so once a project was resident in memory any authenticated user could attach to it — read
  and write — via the WebSocket, MCP `open_project`, or any MCP tool. The check now precedes the
  cache lookup. Invisible with one seeded user; a breach the moment real users exist.
- Project sessions are evicted when idle instead of staying resident for the process lifetime.
- Permissive CORS is applied only under dev auth, not in production alongside a session cookie.
- The WebSocket authenticates with the session cookie same-origin (`?token=` remains for
  cross-origin and non-browser clients), keeping the secret out of proxy logs, and closes with a
  real code + reason instead of a bare frame.
- Auth resolves through one primitive with consistent failures, rather than three code paths with
  three different rejection styles.
- Internal errors no longer leak sqlx error text to the client.

### Changed

- The seeded `developer` user and its `nib-dev-token` are dev-only (`NIB_DEV_AUTH`); production
  creates no default account.

[Unreleased]: https://github.com/eetu/nib/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/eetu/nib/releases/tag/v0.1.0
