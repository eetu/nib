//! Real-SVG round-trip corpus — the Phase-E fidelity gate.
//!
//! nib's byte-preserving serializer must be a **fixed point** on real-world SVG shapes: an
//! *unedited* `parse_svg → serialize_svg` returns the source byte-for-byte, and re-parsing +
//! re-serializing is stable. This corpus is the safety net the full-SVG-DOM rewrite (Phase E)
//! is checked against, and the fidelity gate for finalization. Grow it with any SVG that trips
//! the round-trip.

use nib_core::model::document::{parse_svg, serialize_svg};

const CORPUS: &[(&str, &str)] = &[
    ("minimal", include_str!("fixtures/minimal.svg")),
    ("icon-group", include_str!("fixtures/icon-group.svg")),
    ("gradient", include_str!("fixtures/gradient.svg")),
    (
        "mixed-elements",
        include_str!("fixtures/mixed-elements.svg"),
    ),
    ("style-block", include_str!("fixtures/style-block.svg")),
    ("transforms", include_str!("fixtures/transforms.svg")),
    ("prolog", include_str!("fixtures/prolog.svg")),
    ("shapes", include_str!("fixtures/shapes.svg")),
    ("defs", include_str!("fixtures/defs.svg")),
    ("entities", include_str!("fixtures/entities.svg")),
    ("cdata", include_str!("fixtures/cdata.svg")),
    ("use-symbol", include_str!("fixtures/use-symbol.svg")),
    ("text-tspan", include_str!("fixtures/text-tspan.svg")),
    (
        "doctype-comments",
        include_str!("fixtures/doctype-comments.svg"),
    ),
    ("nested-deep", include_str!("fixtures/nested-deep.svg")),
    ("compact-path", include_str!("fixtures/compact-path.svg")),
    ("inkscape", include_str!("fixtures/inkscape.svg")),
    ("illustrator", include_str!("fixtures/illustrator.svg")),
    ("components", include_str!("fixtures/components.svg")),
    (
        "icon-optimized",
        include_str!("fixtures/icon-optimized.svg"),
    ),
    // A REAL Pixelmator Pro 3.8 export (it round-tripped nib's own exports through the app): top-level
    // <clipPath>, userSpaceOnUse gradients with `1e-05` stop offsets, everything flattened to <path>.
    ("pixelmator", include_str!("fixtures/pixelmator.svg")),
];

/// `<defs>` content (clipPath/mask/gradient/filter contents) is not directly-editable canvas
/// content, so it must NOT project as editable paths — only the two referencing shapes (the rect +
/// the path) do; the `<circle>` inside the `<clipPath>` stays opaque + re-emits verbatim.
#[test]
fn defs_contents_do_not_project_as_editable_paths() {
    let doc = parse_svg(include_str!("fixtures/defs.svg")).unwrap();
    let paths = doc.tree.as_ref().unwrap().project_paths();
    let ids: Vec<&str> = paths.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        ids,
        ["rect-0", "path-1"],
        "only the referencing shapes project: {ids:?}"
    );
}

/// A **component** = a `<g>` directly inside a `<defs>`. Its shapes MUST project as editable paths
/// (so editing the definition propagates to every `<use>`), while other def content (a `<circle>` in
/// a `<clipPath>`) stays opaque — the carve-out is `<g>`-in-`<defs>` only. `<use>` never projects.
#[test]
fn component_def_group_projects_its_shapes_but_other_defs_stay_opaque() {
    let doc = parse_svg(include_str!("fixtures/components.svg")).unwrap();
    let paths = doc.tree.as_ref().unwrap().project_paths();
    // The die's body + 6 pips (inside <defs><g id="die">) project; the two <use> instances do not.
    assert_eq!(
        paths.len(),
        7,
        "die body + 6 pips project: {:?}",
        ids(&paths)
    );
    assert!(
        paths.iter().all(|p| !p.uid.is_empty()),
        "def-shapes carry uids (editable + sync-addressable)"
    );

    // Control: a <circle> inside a <clipPath> (defs.svg) must still NOT project.
    let defs = parse_svg(include_str!("fixtures/defs.svg")).unwrap();
    let dids = ids(&defs.tree.as_ref().unwrap().project_paths());
    assert_eq!(
        dids,
        ["rect-0", "path-1"],
        "clipPath contents stay opaque: {dids:?}"
    );
}

fn ids(paths: &[nib_core::model::types::PathElement]) -> Vec<String> {
    paths.iter().map(|p| p.id.clone()).collect()
}

/// An unedited document must serialize back to its exact source — nib touches only what the
/// user edits; everything else (other elements, defs, comments, whitespace) is verbatim.
#[test]
fn unedited_parse_serialize_is_byte_for_byte() {
    for (name, src) in CORPUS {
        let doc = parse_svg(src).unwrap_or_else(|e| panic!("{name}: parse failed: {e}"));
        let out = serialize_svg(&doc);
        assert_eq!(
            &out, src,
            "\n[{name}] unedited round-trip was not byte-for-byte.\n--- got ---\n{out}\n--- want ---\n{src}\n"
        );
    }
}

/// Serializing is idempotent: parse→serialize twice yields the same bytes (guards against a
/// transformation that keeps changing the document each save, e.g. viewBox drift).
#[test]
fn serialize_is_a_fixed_point() {
    for (name, src) in CORPUS {
        let once = serialize_svg(&parse_svg(src).unwrap());
        let twice = serialize_svg(&parse_svg(&once).unwrap());
        assert_eq!(once, twice, "[{name}] serialize was not idempotent");
    }
}

/// Every fixture parses (no `<svg>` root / malformed markup) — a smoke check as the corpus grows.
#[test]
fn every_fixture_parses() {
    for (name, src) in CORPUS {
        assert!(parse_svg(src).is_ok(), "[{name}] failed to parse");
    }
}

/// Malformed / edge input never panics — bad markup returns `Err` (leaving any current doc
/// untouched, per the SOURCE drawer's fail-safe); it must not crash the engine.
#[test]
fn malformed_input_errors_without_panicking() {
    for bad in [
        "",
        "   ",
        "not svg at all",
        "<div>nope</div>",
        "<svg>",                // unclosed
        "<svg><path d=></svg>", // broken attr
        "<?xml version=\"1.0\"?>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><g></svg>", // mismatched close
    ] {
        assert!(
            parse_svg(bad).is_err(),
            "expected Err (not panic/Ok) for {bad:?}"
        );
    }
    // A degenerate-but-valid empty root parses + round-trips.
    let empty = "<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>";
    assert_eq!(serialize_svg(&parse_svg(empty).unwrap()), empty);
}

/// Nesting past the depth cap is rejected with `Err`, not a stack overflow. The streaming
/// pre-scan runs before roxmltree's (recursive) parse and nib's recursive tree walks, so even
/// 5000-deep — which overflowed the stack before — is caught cheaply.
#[test]
fn deeply_nested_svg_errors_without_overflowing() {
    let mut s = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg">"#);
    for _ in 0..5000 {
        s.push_str("<g>");
    }
    for _ in 0..5000 {
        s.push_str("</g>");
    }
    s.push_str("</svg>");
    assert!(
        parse_svg(&s).is_err(),
        "over-deep nesting must Err, not panic/overflow"
    );
}

/// Editing one path leaves entity-laden siblings byte-for-byte — the surgical splice touches only
/// the edited tag; `&amp;`/`&#169;`/CDATA elsewhere are preserved verbatim.
#[test]
fn editing_preserves_entity_laden_siblings() {
    use nib_core::model::types::Point;
    let src = include_str!("fixtures/entities.svg");
    let mut doc = parse_svg(src).unwrap();
    doc.paths[0].subpaths[0].nodes[1].point = Point { x: 80.0, y: 80.0 };
    doc.paths[0].edited = true;
    let out = serialize_svg(&doc);
    assert!(
        out.contains("Rock &amp; Roll &lt;3&gt; &#169; &#xB5;"),
        "desc verbatim: {out}"
    );
    assert!(
        out.contains(r#"id="a&amp;b""#),
        "edited path keeps its escaped id: {out}"
    );
    assert!(out.contains("L 80 80"), "the edit applied: {out}");
}

/// Editing a path inside a real Inkscape document leaves the producer cruft intact — the
/// `sodipodi:namedview`, the `<metadata>`/RDF block, and the layer `<g>` wrapper all re-emit
/// verbatim; only the edited path's tag changes. This is the byte-preservation promise on the
/// most common real-world SVG producer.
#[test]
fn editing_preserves_inkscape_producer_cruft() {
    use nib_core::model::types::Point;
    let src = include_str!("fixtures/inkscape.svg");
    let mut doc = parse_svg(src).unwrap();
    // path1 is the only editable projected path; nudge one of its anchors.
    let last = doc.paths[0].subpaths[0].nodes.len() - 1;
    doc.paths[0].subpaths[0].nodes[last].point = Point { x: 50.0, y: 50.0 };
    doc.paths[0].edited = true;
    let out = serialize_svg(&doc);
    assert!(
        out.contains(r#"<sodipodi:namedview"#),
        "namedview survives: {out}"
    );
    assert!(
        out.contains(r#"inkscape:current-layer="layer1""#),
        "namedview attrs verbatim: {out}"
    );
    assert!(
        out.contains("<dc:format>image/svg+xml</dc:format>"),
        "RDF metadata verbatim: {out}"
    );
    assert!(
        out.contains(r#"<g inkscape:label="Layer 1" inkscape:groupmode="layer" id="layer1">"#),
        "layer group wrapper verbatim: {out}"
    );
    assert!(
        out.contains(r#"id="path1""#),
        "edited path keeps its id: {out}"
    );
}
