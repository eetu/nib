//! Glyph outlining: a `<text>` label's shaped glyphs → an editable path.
//!
//! Text is the one thing in the document nib can't reshape — a label carries no anchor geometry,
//! so it can't be node-edited, boolean'd, offset, or outlined. Converting it to a path is the
//! escape hatch, and it's destructive by design: the words stop being words.
//!
//! Shaping runs through rustybuzz (HarfBuzz's algorithm), so ligatures, kerning, contextual
//! alternates, RTL and complex scripts come out right rather than as naively concatenated glyphs.
//! The **font bytes are a host resource, not document state** — the browser reads them from the
//! Local Font Access API or a picked file, the backend from fontdb — so nothing here loads or
//! caches a font: callers hand in the bytes for the one conversion they're doing.
//!
//! The resulting `d` travels *inside* the `TextToPath` op, which is what keeps co-editing honest:
//! a peer replaying the op reproduces the exact geometry the author saw, whether or not it has
//! that font installed.

use kurbo::BezPath;
use rustybuzz::ttf_parser::{GlyphId, OutlineBuilder};
use rustybuzz::{Face, UnicodeBuffer};
use serde::{Deserialize, Serialize};

use crate::model::path::{parse_path_d, path_to_d_prec};

/// Where the anchor point sits along the shaped run (SVG `text-anchor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Start,
    Middle,
    End,
}

impl Anchor {
    /// Parse an SVG `text-anchor` value; anything unrecognized reads as `start`, per SVG.
    pub fn parse(value: &str) -> Anchor {
        match value.trim() {
            "middle" => Anchor::Middle,
            "end" => Anchor::End,
            _ => Anchor::Start,
        }
    }
}

/// Everything the outliner needs about one label, resolved from its element (and the ancestors it
/// inherits font properties from) before the font is chosen.
#[derive(Debug, Clone)]
pub struct TextLayout {
    pub text: String,
    /// The anchor point, in the element's own user space (its `x`/`y`, plus `dx`/`dy`).
    pub x: f64,
    pub y: f64,
    pub font_size: f64,
    pub letter_spacing: f64,
    pub anchor: Anchor,
}

/// Accumulates one glyph's outline into a document-space `BezPath`. Font outlines are Y-up in
/// font units; SVG is Y-down in user units, so every point is scaled and flipped about the
/// baseline as it arrives.
struct Outliner {
    path: BezPath,
    pen_x: f64,
    pen_y: f64,
    scale: f64,
    open: bool,
}

impl Outliner {
    fn map(&self, x: f32, y: f32) -> kurbo::Point {
        kurbo::Point::new(
            self.pen_x + f64::from(x) * self.scale,
            self.pen_y - f64::from(y) * self.scale,
        )
    }
}

impl OutlineBuilder for Outliner {
    fn move_to(&mut self, x: f32, y: f32) {
        // A glyph contour that never closed itself still has to close — an open contour would
        // render as a stroke-shaped sliver instead of filled ink.
        if self.open {
            self.path.close_path();
        }
        self.path.move_to(self.map(x, y));
        self.open = true;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to(self.map(x, y));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.path.quad_to(self.map(x1, y1), self.map(x, y));
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.path
            .curve_to(self.map(x1, y1), self.map(x2, y2), self.map(x, y));
    }

    fn close(&mut self) {
        self.path.close_path();
        self.open = false;
    }
}

/// Which face inside `font` is the one named `postscript_name` — the answer matters because a
/// system font is often a *collection* (`Helvetica.ttc` holds regular, bold, italic, …) and the
/// browser's Local Font Access API hands over the whole collection plus the PostScript name of the
/// face the author asked for. Falls back to 0 (a plain `.ttf`/`.otf`, or a name that isn't in
/// there), which is also what a caller with no name should pass.
pub fn face_index_for(font: &[u8], postscript_name: &str) -> u32 {
    let count = rustybuzz::ttf_parser::fonts_in_collection(font).unwrap_or(1);
    (0..count)
        .find(|&i| {
            rustybuzz::ttf_parser::Face::parse(font, i).is_ok_and(|face| {
                face.names().into_iter().any(|name| {
                    // name_id 6 is the PostScript name.
                    name.name_id == 6 && name.to_string().as_deref() == Some(postscript_name)
                })
            })
        })
        .unwrap_or(0)
}

/// One face inside a font file — what a host needs to choose between them when nothing else names
/// the one it wants (a picked `.ttc` arrives as bytes and a filename, no more).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaceInfo {
    /// Pass as `outline_d`'s `face_index`.
    pub index: u32,
    /// Family name (`Liberation Sans`), empty if the face declares none.
    pub family: String,
    /// Style within the family (`Bold Italic`).
    pub style: String,
    /// OS/2 weight class, 400 = regular, 700 = bold.
    pub weight: u16,
    pub italic: bool,
}

/// Read the name-table entry `name_id` off a face.
fn face_name(face: &rustybuzz::ttf_parser::Face, name_id: u16) -> Option<String> {
    face.names()
        .into_iter()
        .find(|n| n.name_id == name_id)
        .and_then(|n| n.to_string())
}

/// Every face in these bytes. Empty when they aren't a font nib can read — which is also the
/// cheapest way for a caller to tell a `.woff2` (compressed) from a raw `.ttf`/`.otf`/`.ttc`
/// *before* it tries to shape anything with it.
pub fn faces_in(font: &[u8]) -> Vec<FaceInfo> {
    let count = rustybuzz::ttf_parser::fonts_in_collection(font).unwrap_or(1);
    (0..count)
        .filter_map(|index| {
            let face = rustybuzz::ttf_parser::Face::parse(font, index).ok()?;
            Some(FaceInfo {
                index,
                // Name ids 1/2 are the family and its style; 16/17 are the typographic pair, which
                // is the one that tells "Light" apart from "Regular" in a large family.
                family: face_name(&face, 16)
                    .or_else(|| face_name(&face, 1))
                    .unwrap_or_default(),
                style: face_name(&face, 17)
                    .or_else(|| face_name(&face, 2))
                    .unwrap_or_default(),
                weight: face.weight().to_number(),
                italic: face.is_italic() || face.is_oblique(),
            })
        })
        .collect()
}

/// Shape `layout`'s text with the given font and return the glyph outlines as a path `d` — the
/// geometry the `TextToPath` op carries. `face_index` picks a face out of a collection (`.ttc`);
/// 0 for a plain `.ttf`/`.otf`.
///
/// `None` when the bytes aren't a font nib can read (a `.woff2` is compressed, not a raw face) or
/// the run produces no ink at all (empty/whitespace text, every glyph blank).
pub fn outline_d(
    font: &[u8],
    face_index: u32,
    layout: &TextLayout,
    precision: usize,
) -> Option<String> {
    let face = Face::from_slice(font, face_index)?;
    let upem = f64::from(face.units_per_em());
    if upem <= 0.0 || layout.font_size <= 0.0 {
        return None;
    }
    let scale = layout.font_size / upem;

    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(&layout.text);
    // Script/direction/language read off the text itself — what makes an RTL run lay out RTL.
    buffer.guess_segment_properties();
    let glyphs = rustybuzz::shape(&face, &[], buffer);

    let positions = glyphs.glyph_positions();
    let infos = glyphs.glyph_infos();
    if infos.is_empty() {
        return None;
    }

    // The run's total advance decides where `text-anchor: middle/end` starts drawing. Letter
    // spacing lands *between* glyphs, so it counts one fewer time than there are glyphs.
    let advance: f64 = positions
        .iter()
        .map(|p| f64::from(p.x_advance) * scale)
        .sum::<f64>()
        + layout.letter_spacing * (infos.len().saturating_sub(1)) as f64;
    let start_x = match layout.anchor {
        Anchor::Start => layout.x,
        Anchor::Middle => layout.x - advance / 2.0,
        Anchor::End => layout.x - advance,
    };

    let mut out = Outliner {
        path: BezPath::new(),
        pen_x: 0.0,
        pen_y: 0.0,
        scale,
        open: false,
    };
    let mut cursor_x = start_x;
    let mut cursor_y = layout.y;
    for (info, pos) in infos.iter().zip(positions.iter()) {
        out.pen_x = cursor_x + f64::from(pos.x_offset) * scale;
        out.pen_y = cursor_y - f64::from(pos.y_offset) * scale;
        out.open = false;
        // A blank glyph (space) outlines to nothing — it still advances the pen.
        face.outline_glyph(GlyphId(info.glyph_id as u16), &mut out);
        if out.open {
            out.path.close_path();
            out.open = false;
        }
        cursor_x += f64::from(pos.x_advance) * scale + layout.letter_spacing;
        cursor_y -= f64::from(pos.y_advance) * scale;
    }

    let d = out.path.to_svg();
    if d.is_empty() {
        return None;
    }
    // Round-trip through the model's own parser/serializer so the `d` an outline produces is
    // shaped exactly like every other path nib writes (absolute cubics, one precision).
    let subpaths = parse_path_d(&d);
    if subpaths.is_empty() {
        return None;
    }
    Some(path_to_d_prec(&subpaths, precision))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::geometry::subpaths_bounds;

    /// Any real outline font off the host, via the same system-font database the backend resolves
    /// with. Returns `None` on a machine with no fonts at all, and every test here then no-ops
    /// rather than failing on the environment.
    fn test_font() -> Option<Vec<u8>> {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let id = db.faces().find(|f| f.index == 0)?.id;
        db.with_face_data(id, |data, _| data.to_vec())
    }

    fn layout(text: &str, anchor: Anchor) -> TextLayout {
        TextLayout {
            text: text.to_string(),
            x: 10.0,
            y: 50.0,
            font_size: 40.0,
            letter_spacing: 0.0,
            anchor,
        }
    }

    #[test]
    fn outlines_a_label_into_ink_at_the_anchor() {
        let Some(font) = test_font() else { return };
        let d = outline_d(&font, 0, &layout("Hi", Anchor::Start), 3).expect("outlined");
        let subpaths = parse_path_d(&d);
        assert!(!subpaths.is_empty(), "produced geometry");
        let b = subpaths_bounds(&subpaths).expect("bounds");
        // Ink starts at the anchor and sits above the baseline (SVG y grows downward).
        assert!(b.min_x >= 9.0, "ink starts at the anchor x: {}", b.min_x);
        assert!(
            b.max_y <= 51.0,
            "ink sits on/above the baseline: {}",
            b.max_y
        );
        assert!(b.min_y > 0.0 && b.min_y < 50.0, "cap height above baseline");
    }

    #[test]
    fn text_anchor_shifts_the_run() {
        let Some(font) = test_font() else { return };
        let bounds = |a: Anchor| {
            let d = outline_d(&font, 0, &layout("Hi", a), 3).unwrap();
            subpaths_bounds(&parse_path_d(&d)).unwrap()
        };
        let start = bounds(Anchor::Start);
        let middle = bounds(Anchor::Middle);
        let end = bounds(Anchor::End);
        let width = start.max_x - start.min_x;
        assert!(
            middle.min_x < start.min_x && end.min_x < middle.min_x,
            "start > middle > end"
        );
        // `end` puts the whole run left of the anchor; `middle` splits it.
        assert!(
            (end.max_x - 10.0).abs() < width * 0.2,
            "end run finishes at the anchor"
        );
    }

    #[test]
    fn letter_spacing_widens_the_run() {
        let Some(font) = test_font() else { return };
        let tight = outline_d(&font, 0, &layout("Hi", Anchor::Start), 3).unwrap();
        let mut spaced_layout = layout("Hi", Anchor::Start);
        spaced_layout.letter_spacing = 10.0;
        let spaced = outline_d(&font, 0, &spaced_layout, 3).unwrap();
        let w = |d: &str| {
            let b = subpaths_bounds(&parse_path_d(d)).unwrap();
            b.max_x - b.min_x
        };
        assert!(
            w(&spaced) - w(&tight) > 9.0,
            "one 10-unit gap between two glyphs"
        );
    }

    #[test]
    fn blank_and_bogus_input_outline_to_nothing() {
        let Some(font) = test_font() else { return };
        assert!(outline_d(&font, 0, &layout("   ", Anchor::Start), 3).is_none());
        assert!(outline_d(&font, 0, &layout("", Anchor::Start), 3).is_none());
        assert!(outline_d(b"not a font", 0, &layout("Hi", Anchor::Start), 3).is_none());
    }

    #[test]
    fn faces_in_lists_what_a_file_holds() {
        let Some(font) = test_font() else { return };
        let faces = faces_in(&font);
        assert!(!faces.is_empty(), "a real font has at least one face");
        // Indices are exactly the ones `outline_d` accepts, and each face names itself.
        for (i, face) in faces.iter().enumerate() {
            assert_eq!(face.index as usize, i, "index matches position: {face:?}");
            assert!(!face.family.is_empty(), "family named: {face:?}");
            assert!(
                (100..=1000).contains(&face.weight),
                "plausible OS/2 weight: {face:?}"
            );
            assert!(
                outline_d(&font, face.index, &layout("Hi", Anchor::Start), 3).is_some(),
                "every listed face shapes: {face:?}"
            );
        }
    }

    #[test]
    fn faces_in_rejects_what_cannot_be_shaped() {
        // The cheap pre-check a host makes before offering to outline: no faces, no shaping. A
        // compressed `.woff2` lands here too — it starts `wOF2` and holds no readable tables.
        assert!(faces_in(b"not a font").is_empty());
        assert!(faces_in(b"wOF2\x00\x01\x00\x00").is_empty());
    }
}
