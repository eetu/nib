//! Render-fidelity gate — does the **canonical** export look the same as the source?
//!
//! `roundtrip.rs` proves the *byte-preserving* serializer is a fixed point. But the default save is
//! now the **canonical** export (`to_svg` → `serialize_canonical`), which regenerates every element
//! from the model rather than re-emitting verbatim spans. Its failure mode is therefore *visual*, not
//! textual: a dropped `xmlns`, a mangled namespaced attr, a primitive that `refit` wrong, a
//! gradient/clip that stopped resolving — none of which a string compare would flag as "wrong", only
//! as "different". So this rasterizes SOURCE vs. canonical export with **resvg** (the same renderer
//! the backend's MCP `render_document` uses) and asserts they're pixel-equivalent within an
//! anti-aliasing tolerance.
//!
//! This is the automatable half of the 1.0 export-fidelity gate; the manual half is opening a few
//! exports in a real design app. Grow `CORPUS` with any real-world SVG whose export drifts.
//!
//! Note: text glyphs aren't rendered (no fonts loaded) — resvg draws nothing for `<text>` without a
//! fontdb, so text fixtures compare blank-vs-blank here and are covered by the manual pass instead.
//! That keeps the gate deterministic across machines/CI (no dependence on installed fonts).

mod common;
use common::{diff_fraction, rasterize, size_of};
use nib_core::model::document::{parse_svg, serialize_canonical};

/// The full round-trip corpus, reused for render-diffing, plus fixtures aimed at canonical-export
/// risk not covered elsewhere (group-inherited fill, clip-path nesting + rounded rect, radial +
/// stop-opacity gradients).
const CORPUS: &[(&str, &str)] = &[
    ("minimal", include_str!("fixtures/minimal.svg")),
    ("icon-group", include_str!("fixtures/icon-group.svg")),
    ("gradient", include_str!("fixtures/gradient.svg")),
    ("mixed-elements", include_str!("fixtures/mixed-elements.svg")),
    ("style-block", include_str!("fixtures/style-block.svg")),
    ("transforms", include_str!("fixtures/transforms.svg")),
    ("prolog", include_str!("fixtures/prolog.svg")),
    ("shapes", include_str!("fixtures/shapes.svg")),
    ("defs", include_str!("fixtures/defs.svg")),
    ("entities", include_str!("fixtures/entities.svg")),
    ("cdata", include_str!("fixtures/cdata.svg")),
    ("use-symbol", include_str!("fixtures/use-symbol.svg")),
    ("text-tspan", include_str!("fixtures/text-tspan.svg")),
    ("doctype-comments", include_str!("fixtures/doctype-comments.svg")),
    ("nested-deep", include_str!("fixtures/nested-deep.svg")),
    ("compact-path", include_str!("fixtures/compact-path.svg")),
    ("inkscape", include_str!("fixtures/inkscape.svg")),
    ("illustrator", include_str!("fixtures/illustrator.svg")),
    ("components", include_str!("fixtures/components.svg")),
    ("icon-optimized", include_str!("fixtures/icon-optimized.svg")),
    // Canonical-export stress additions:
    ("svgo-oneline", include_str!("fixtures/svgo-oneline.svg")),
    ("figma-export", include_str!("fixtures/figma-export.svg")),
    ("gradient-radial", include_str!("fixtures/gradient-radial.svg")),
    // A real Pixelmator Pro export — the manual design-app pass, folded back in as an automated
    // producer fixture (top-level clipPath, userSpaceOnUse gradients, all-paths).
    ("pixelmator", include_str!("fixtures/pixelmator.svg")),
];

/// Longest-side render target: big enough that a real defect covers many pixels, small enough to
/// stay fast across the whole corpus.
const TARGET: f32 = 480.0;
/// Per-pixel channel delta above which a pixel counts as "different" — absorbs the sub-pixel AA
/// that coordinate rounding (precision 3) shifts along edges.
const CHANNEL_TOL: i16 = 24;
/// Max fraction of differing pixels tolerated per fixture. An *unedited* import→canonical should be
/// near-identical, so this is headroom for edge AA, not a fudge factor.
const MAX_DIFF: f64 = 0.02;

/// The gate: for every corpus SVG, the canonical export must rasterize to (near-)the-same pixels as
/// the source. Reports *all* drifting fixtures at once (with the offending export inlined) rather
/// than failing on the first, so a regression sweep shows the whole blast radius.
#[test]
fn canonical_export_is_render_equivalent_to_source() {
    let mut failures = Vec::new();
    for (name, src) in CORPUS {
        let Some((w, h)) = size_of(src) else {
            continue; // no drawable area — nothing to render-diff
        };
        let scale = TARGET / w.max(h);
        let pw = (w * scale).round().max(1.0) as u32;
        let ph = (h * scale).round().max(1.0) as u32;

        let doc = parse_svg(src).unwrap_or_else(|e| panic!("[{name}] parse failed: {e}"));
        let tree = doc.tree.as_ref().expect("a parsed doc always carries its tree");
        let canonical = serialize_canonical(&doc, tree, 3);

        let source_px =
            rasterize(src, pw, ph).unwrap_or_else(|| panic!("[{name}] source did not rasterize"));
        let export_px = rasterize(&canonical, pw, ph).unwrap_or_else(|| {
            panic!("[{name}] canonical export did not rasterize:\n{canonical}")
        });

        let frac = diff_fraction(&source_px, &export_px, CHANNEL_TOL);
        if frac > MAX_DIFF {
            failures.push(format!(
                "  [{name}] {:.2}% of pixels differ (tolerance {:.2}%)\n--- canonical export ---\n{canonical}",
                frac * 100.0,
                MAX_DIFF * 100.0,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "canonical export drifted from the source render on {} fixture(s):\n{}",
        failures.len(),
        failures.join("\n"),
    );
}
