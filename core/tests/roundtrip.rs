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
];

/// `<defs>` content (clipPath/mask/gradient/filter contents) is not directly-editable canvas
/// content, so it must NOT project as editable paths — only the two referencing shapes (the rect +
/// the path) do; the `<circle>` inside the `<clipPath>` stays opaque + re-emits verbatim.
#[test]
fn defs_contents_do_not_project_as_editable_paths() {
    let doc = parse_svg(include_str!("fixtures/defs.svg")).unwrap();
    let paths = doc.tree.as_ref().unwrap().project_paths();
    let ids: Vec<&str> = paths.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["rect-0", "path-1"], "only the referencing shapes project: {ids:?}");
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
        "<svg>",             // unclosed
        "<svg><path d=></svg>", // broken attr
        "<?xml version=\"1.0\"?>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><g></svg>", // mismatched close
    ] {
        assert!(parse_svg(bad).is_err(), "expected Err (not panic/Ok) for {bad:?}");
    }
    // A degenerate-but-valid empty root parses + round-trips.
    let empty = "<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>";
    assert_eq!(serialize_svg(&parse_svg(empty).unwrap()), empty);
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
    assert!(out.contains("Rock &amp; Roll &lt;3&gt; &#169; &#xB5;"), "desc verbatim: {out}");
    assert!(out.contains(r#"id="a&amp;b""#), "edited path keeps its escaped id: {out}");
    assert!(out.contains("L 80 80"), "the edit applied: {out}");
}
