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
];

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
