---
name: nib-design
description: Visual identity for nib — a sibling in eetu's homebrew web app family. Layers nib's glyph, wordmark, layout, and voice on top of the shared halo-design tokens. Use when building or styling nib's UI.
user-invocable: true
---

# nib-design

Shared tokens + conventions come from `halo-design` — copy `colors_and_type.css`
verbatim (already at `frontend/src/lib/styles/halo.css`) and use the `--halo-*`
vars in Svelte `<style>` blocks. Below is nib's delta.

## The four deltas

**Glyph** — an **editable bezier node**. 64×64, opaque white tile, dark
theme-text strokes (`#525252`); the one hardcoded brand color is the warm
`#f78f08` active-anchor dot (the family "warm centre"). A curve with two hollow
square end anchors and, on the mid anchor, the classic tangent control handle
(a line with round knobs). Stroke ~3 (curve) / ~2.5 (handle + knobs), round
caps. Source: `frontend/static/favicon.svg` (+ `icon-maskable.svg`); PNGs via
`frontend/scripts/gen-icons.sh`.

**Wordmark** — `nib` + accent period. Full riff: *"mightier than the prose.
nib."* — the pen vs. the sword, and prose is exactly what you *can't* finetune
an SVG with. Collapses to bare `nib.` under ~720px. Lowercase, Inter 600,
`-0.04em`. See `frontend/src/lib/components/Wordmark.svelte`.

**Layout / density** — a **desktop tool**, dense, dark-friendly. Thin top bar
(wordmark · open/undo/redo · filename · save/copy) · a body row of: optional
**file list** (opened folder) → slim **tool rail** (Lucide icons) → full-bleed
**editor canvas** with a collapsible **source** drawer → **inspector** (snap
settings, selected-node coords, path list). The canvas is the hero: a checker
backdrop, artwork drawn to scale, editing overlay (anchors/handles/snap ring)
in constant-size screen space on top.

**Voice** — terse, lowercase, geometry-forward. Section labels in Space Grotesk,
uppercase, tracked. Numbers/coords do the talking. No marketing tone, no emoji.
Empty states get one quiet line (`no node selected`, `no svg loaded`).

## Differences from halo / chat / scribe / ocular

| | nib |
|---|---|
| Stack | **frontend-only** SvelteKit SPA (no backend yet — client-side editor) |
| Glyph | bezier node + warm active-anchor dot |
| Hero element | the interactive canvas (drag anchors/handles, close loops by snap) |
| Accent use | the selected node, snap ring, and active tool light up `--halo-accent` |
| Filesystem | edits a folder of SVGs in place via the File System Access API |

## Source-of-truth files

- `frontend/src/lib/styles/halo.css` — canonical tokens (verbatim copy).
- `frontend/src/lib/components/Wordmark.svelte` — brand.
- `frontend/src/lib/components/{EditorCanvas,Overlay,ToolRail,Inspector,TopBar,SourceView,FileList}.svelte` — the screen.
- `frontend/static/favicon.svg` + `icon-maskable.svg` — the glyph; `scripts/gen-icons.sh` regenerates the PNGs.
