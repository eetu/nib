//! Pixel-verify the geometry/effect tools end-to-end: apply the op the tool emits, render the
//! canonical export with resvg, and assert the pixels actually changed the way the tool promises —
//! not just that the model mutated (ops.rs unit-tests the model; this proves it *renders*).
//!
//! Covered here (deterministic, font-free): rotate, flip, rounded-rect, drop-shadow. Not here:
//! **text** (resvg needs system fonts to raster glyphs — that's the manual pass), and **eyedropper**
//! (a frontend pixel-sample → set-fill, covered by the Playwright e2e "eyedropper samples one
//! shape's fill"). Both are noted in the roadmap's pixel-verify item.

mod common;
use common::{ink_bbox, left_right_ink, pixel_at, render_fit};
use nib_core::model::document::{parse_svg, serialize_canonical};
use nib_core::model::types::SvgDocument;
use nib_core::ops::{Op, ShapeSpec, apply};

/// Parse an SVG and seed the working paths from its tree (imported primitives become editable
/// paths), mirroring how the editor/backend hydrate a document before applying ops.
fn doc(svg: &str) -> SvgDocument {
    let mut d = parse_svg(svg).unwrap();
    d.paths = d.tree.as_ref().unwrap().project_paths();
    d
}

/// Canonically export `d` and rasterize it at a fixed 300px longest side → `(rgba, w, h)`.
fn render(d: &SvgDocument) -> (Vec<u8>, u32, u32) {
    let svg = serialize_canonical(d, d.tree.as_ref().unwrap(), 3);
    render_fit(&svg, 300.0)
}

/// Is the pixel at `(x, y)` ink (not the white backdrop)?
fn inked(px: &[u8], w: u32, x: u32, y: u32) -> bool {
    let p = pixel_at(px, w, x, y);
    p[0] < 235 || p[1] < 235 || p[2] < 235
}

#[test]
fn rotate_90_swaps_a_portrait_shape_to_landscape() {
    // A tall, narrow bar.
    let mut d = doc(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="42" y="18" width="16" height="64" fill="#111111"/></svg>"##,
    );
    let (before, w, h) = render(&d);
    let (bx0, by0, bx1, by1) = ink_bbox(&before, w, h).expect("bar renders");
    assert!(by1 - by0 > bx1 - bx0, "starts portrait: {bx0},{by0}..{bx1},{by1}");

    assert!(apply(
        &mut d,
        &Op::RotatePath {
            path: 0,
            degrees: 90.0,
            cx: None,
            cy: None,
        }
    ));
    let (after, w2, h2) = render(&d);
    let (ax0, ay0, ax1, ay1) = ink_bbox(&after, w2, h2).expect("rotated bar renders");
    assert!(ax1 - ax0 > ay1 - ay0, "rotates to landscape: {ax0},{ay0}..{ax1},{ay1}");

    // Rotating about its own centre keeps the centre roughly put (not translated off).
    let before_cx = (bx0 + bx1) / 2;
    let after_cx = (ax0 + ax1) / 2;
    assert!(
        (before_cx as i32 - after_cx as i32).abs() < 10,
        "centre stays put: {before_cx} vs {after_cx}"
    );
}

#[test]
fn flip_horizontal_mirrors_mass_across_the_centre() {
    // A right triangle whose bulk is on the LEFT (tall vertical edge at x=20, apex at the right).
    let mut d = doc(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path d="M 20 20 L 20 80 L 80 50 Z" fill="#111111"/></svg>"##,
    );
    let (before, w, h) = render(&d);
    let (bl, br) = left_right_ink(&before, w, h);
    assert!(bl > br * 2, "starts left-heavy: {bl} vs {br}");
    let bbox_before = ink_bbox(&before, w, h).unwrap();

    assert!(apply(
        &mut d,
        &Op::FlipPath {
            path: 0,
            horizontal: true,
            cx: None,
            cy: None,
        }
    ));
    let (after, w2, h2) = render(&d);
    let (al, ar) = left_right_ink(&after, w2, h2);
    assert!(ar > al * 2, "flips right-heavy: {al} vs {ar}");

    // A horizontal flip about the shape's centre preserves the bounding box (±AA).
    let bbox_after = ink_bbox(&after, w2, h2).unwrap();
    let close = |a: u32, b: u32| (a as i32 - b as i32).abs() <= 2;
    assert!(
        close(bbox_after.0, bbox_before.0) && close(bbox_after.2, bbox_before.2),
        "flip preserves horizontal bounds: {bbox_before:?} vs {bbox_after:?}"
    );
}

#[test]
fn rounded_rect_hollows_the_corners() {
    // A big rect; probe just inside the top-left corner.
    let mut d = doc(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="10" y="10" width="80" height="80" fill="#111111"/></svg>"##,
    );
    let (sharp, w, _h) = render(&d);
    let scale = w as f32 / 100.0;
    let (px, py) = ((14.0 * scale) as u32, (14.0 * scale) as u32);
    assert!(inked(&sharp, w, px, py), "sharp corner is filled");

    // Round the corners hard — the corner region (>r from the arc centre) becomes background.
    assert!(apply(
        &mut d,
        &Op::SetShape {
            path: 0,
            subpath: 0,
            spec: ShapeSpec::Rect {
                x0: 10.0,
                y0: 10.0,
                x1: 90.0,
                y1: 90.0,
                rx: 18.0,
                ry: 18.0,
            },
        }
    ));
    let (rounded, w2, _h2) = render(&d);
    assert!(
        !inked(&rounded, w2, px, py),
        "rounded corner is hollow (background shows through)"
    );
    // Sanity: the centre is still filled, so we didn't just erase the shape.
    let c = (50.0 * scale) as u32;
    assert!(inked(&rounded, w2, c, c), "centre still filled after rounding");
}

#[test]
fn drop_shadow_paints_ink_beyond_the_shape() {
    let mut d = doc(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="30" y="30" width="20" height="20" fill="#3b82f6"/></svg>"##,
    );
    let (before, w, h) = render(&d);
    let (_, _, bx1, by1) = ink_bbox(&before, w, h).expect("rect renders");

    assert!(apply(
        &mut d,
        &Op::SetDropShadow {
            path: 0,
            dx: 12.0,
            dy: 12.0,
            blur: 2.0,
            color: "#000000".into(),
            opacity: 1.0,
            id: "sh".into(),
            uid: Some("fx".into()),
        }
    ));
    let (after, w2, h2) = render(&d);
    let (_, _, ax1, ay1) = ink_bbox(&after, w2, h2).expect("shape + shadow render");
    assert!(
        ax1 > bx1 + 4 && ay1 > by1 + 4,
        "shadow extends down-right past the shape: before max ({bx1},{by1}), after ({ax1},{ay1})"
    );
}
