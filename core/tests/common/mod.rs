//! Shared test helpers: rasterize an SVG string with resvg (the same renderer as the backend's MCP
//! `render_document`) and inspect the resulting pixels. Used by `fidelity.rs` (source-vs-export
//! render equivalence) and `tool_pixels.rs` (a tool's op produces the visual change it promises).
//!
//! A file under `tests/common/` is a shared module, not a test target of its own.
//!
//! Each test binary (`fidelity`, `tool_pixels`) inlines this module and uses a different subset of
//! the helpers, so unused-fn warnings here are expected — silence them module-wide.
#![allow(dead_code)]

use resvg::{tiny_skia, usvg};

/// The intrinsic pixel size resvg derives for an SVG (viewBox / width-height), or `None` if it won't
/// parse or has no drawable area.
pub fn size_of(svg: &str) -> Option<(f32, f32)> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let s = tree.size();
    (s.width() > 0.0 && s.height() > 0.0).then(|| (s.width(), s.height()))
}

/// Rasterize `svg` onto a `pw`×`ph` white canvas (its own size scaled to fill), returning RGBA bytes.
/// `None` if resvg can't parse it. (No fonts loaded — `<text>` renders nothing, deterministically.)
pub fn rasterize(svg: &str, pw: u32, ph: u32) -> Option<Vec<u8>> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let size = tree.size();
    if size.width() <= 0.0 || size.height() <= 0.0 {
        return None;
    }
    let mut pixmap = tiny_skia::Pixmap::new(pw, ph)?;
    pixmap.fill(tiny_skia::Color::WHITE);
    let sx = pw as f32 / size.width();
    let sy = ph as f32 / size.height();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(sx, sy),
        &mut pixmap.as_mut(),
    );
    Some(pixmap.data().to_vec())
}

/// Rasterize at a fixed longest-side `target`, returning `(rgba, width, height)`. Panics with the
/// SVG inlined if it won't render — a test that can't rasterize its own input has a bug, not a diff.
pub fn render_fit(svg: &str, target: f32) -> (Vec<u8>, u32, u32) {
    let (w, h) = size_of(svg).unwrap_or_else(|| panic!("no drawable size:\n{svg}"));
    let scale = target / w.max(h);
    let pw = (w * scale).round().max(1.0) as u32;
    let ph = (h * scale).round().max(1.0) as u32;
    let px = rasterize(svg, pw, ph).unwrap_or_else(|| panic!("did not rasterize:\n{svg}"));
    (px, pw, ph)
}

/// Fraction of pixels whose worst RGB channel differs by more than `tol` between two same-size buffers.
pub fn diff_fraction(a: &[u8], b: &[u8], tol: i16) -> f64 {
    assert_eq!(a.len(), b.len(), "pixmaps differ in size");
    let px = a.len() / 4;
    let mut differing = 0usize;
    for i in 0..px {
        let o = i * 4;
        let worst = (0..3)
            .map(|c| (a[o + c] as i16 - b[o + c] as i16).abs())
            .max()
            .unwrap_or(0);
        if worst > tol {
            differing += 1;
        }
    }
    differing as f64 / px as f64
}

/// A pixel counts as "ink" if it's meaningfully darker/more-saturated than the white backdrop.
pub fn is_ink(px: &[u8], i: usize) -> bool {
    let o = i * 4;
    px[o] < 235 || px[o + 1] < 235 || px[o + 2] < 235
}

/// The bounding box `(min_x, min_y, max_x, max_y)` of ink pixels in a `w`×`h` RGBA buffer, or `None`
/// if the canvas is all white. Max is inclusive.
pub fn ink_bbox(px: &[u8], w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    let (mut minx, mut miny, mut maxx, mut maxy) = (u32::MAX, u32::MAX, 0u32, 0u32);
    let mut any = false;
    for y in 0..h {
        for x in 0..w {
            if is_ink(px, (y * w + x) as usize) {
                any = true;
                minx = minx.min(x);
                miny = miny.min(y);
                maxx = maxx.max(x);
                maxy = maxy.max(y);
            }
        }
    }
    any.then_some((minx, miny, maxx, maxy))
}

/// Count of ink pixels in the left vs. right half of a `w`×`h` buffer — used to detect a horizontal
/// mirror (an asymmetric shape's mass swaps sides).
pub fn left_right_ink(px: &[u8], w: u32, h: u32) -> (usize, usize) {
    let (mut left, mut right) = (0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            if is_ink(px, (y * w + x) as usize) {
                if x < w / 2 {
                    left += 1;
                } else {
                    right += 1;
                }
            }
        }
    }
    (left, right)
}

/// RGBA of the pixel at `(x, y)` in a `w`-wide buffer.
pub fn pixel_at(px: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let o = ((y * w + x) * 4) as usize;
    [px[o], px[o + 1], px[o + 2], px[o + 3]]
}
