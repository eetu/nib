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

/// One run of a multi-part label (a `<tspan>`, or the text around them). Same as a
/// [`TextLayout`] except the position is optional: a run that states none continues from wherever
/// the previous run's pen ended up, which is what makes `<text>a<tspan>b</tspan></text>` read as
/// "ab" rather than stacking both at the same origin.
#[derive(Debug, Clone)]
pub struct RunLayout {
    pub text: String,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub font_size: f64,
    pub letter_spacing: f64,
    pub anchor: Anchor,
}

impl From<&TextLayout> for RunLayout {
    fn from(l: &TextLayout) -> Self {
        RunLayout {
            text: l.text.clone(),
            x: Some(l.x),
            y: Some(l.y),
            font_size: l.font_size,
            letter_spacing: l.letter_spacing,
            anchor: l.anchor,
        }
    }
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
    outline_runs_d(font, face_index, &[layout.into()], precision)
}

/// Shape a label's `runs` — a flat `<text>` is one, a tspan label is several — into a single path
/// `d`. Runs lay out left to right in document order, each starting where it says to and otherwise
/// continuing from the previous run's pen, so a multi-line label keeps its lines and an inline
/// tspan keeps its place in the sentence.
///
/// One font shapes every run. A tspan asking for a different family is shaped in this one anyway
/// (see `TextInfo::foreign_families`, which lets the caller say so) — the alternative is refusing
/// to convert the label at all, which is worse for the common case where the tspan only moved.
pub fn outline_runs_d(
    font: &[u8],
    face_index: u32,
    runs: &[RunLayout],
    precision: usize,
) -> Option<String> {
    let face = Face::from_slice(font, face_index)?;
    let upem = f64::from(face.units_per_em());
    if upem <= 0.0 {
        return None;
    }

    let mut out = Outliner {
        path: BezPath::new(),
        pen_x: 0.0,
        pen_y: 0.0,
        scale: 1.0,
        open: false,
    };
    // The pen carries across runs; a run with its own x/y moves it first.
    let mut cursor_x = 0.0;
    let mut cursor_y = 0.0;

    for run in runs {
        if run.font_size <= 0.0 {
            continue;
        }
        let scale = run.font_size / upem;
        out.scale = scale;

        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(&run.text);
        // Script/direction/language read off the text itself — what makes an RTL run lay out RTL.
        buffer.guess_segment_properties();
        let glyphs = rustybuzz::shape(&face, &[], buffer);
        let positions = glyphs.glyph_positions();
        let infos = glyphs.glyph_infos();
        if infos.is_empty() {
            continue;
        }

        // This run's total advance decides where `text-anchor: middle/end` starts it. Letter
        // spacing lands *between* glyphs, so it counts one fewer time than there are glyphs. Each
        // positioned run is its own anchored chunk, which is exactly how SVG anchors a tspan that
        // states an x.
        let advance: f64 = positions
            .iter()
            .map(|p| f64::from(p.x_advance) * scale)
            .sum::<f64>()
            + run.letter_spacing * (infos.len().saturating_sub(1)) as f64;
        let origin_x = run.x.unwrap_or(cursor_x);
        cursor_y = run.y.unwrap_or(cursor_y);
        cursor_x = match run.anchor {
            Anchor::Start => origin_x,
            Anchor::Middle => origin_x - advance / 2.0,
            Anchor::End => origin_x - advance,
        };

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
            cursor_x += f64::from(pos.x_advance) * scale + run.letter_spacing;
            cursor_y -= f64::from(pos.y_advance) * scale;
        }
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

    /// Multi-line text is how every design tool exports it: one `<tspan>` per line, each stating
    /// its own x/y. Those lines have to survive the conversion as lines.
    #[test]
    fn a_tspan_label_outlines_as_positioned_runs() {
        use crate::model::document::parse_svg;
        let Some(font) = test_font() else { return };
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200"><text x="10" y="40" font-size="20"><tspan x="10" y="40">first</tspan><tspan x="10" y="80">second</tspan></text></svg>"##;
        let doc = parse_svg(svg).unwrap();
        let tree = doc.tree.as_ref().unwrap();
        let info = tree.text_infos().first().cloned().expect("a label");

        assert_eq!(info.text, "firstsecond", "the label reads as one string");
        assert_eq!(info.runs.len(), 2, "one run per tspan: {:?}", info.runs);
        assert_eq!(info.runs[1].y, Some(80.0), "the second line keeps its y");

        let d = outline_runs_d(&font, 0, &info.layout(), 3).expect("outlined");
        let bounds = crate::model::geometry::subpaths_bounds(&parse_path_d(&d)).expect("bounds");
        // Two lines 40 units apart at size 20 — the ink spans both, not one line's worth.
        assert!(
            bounds.max_y - bounds.min_y > 40.0,
            "ink covers both lines: {bounds:?}"
        );
        assert!(
            bounds.min_y > 10.0 && bounds.max_y < 85.0,
            "…and only those: {bounds:?}"
        );
    }

    /// An unpositioned tspan continues the sentence rather than restarting it — the difference
    /// between "Hello world" and both words stacked on the same origin.
    #[test]
    fn an_unpositioned_tspan_continues_from_the_pen() {
        use crate::model::document::parse_svg;
        let Some(font) = test_font() else { return };
        let one_line = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200"><text x="10" y="40" font-size="20">Hello <tspan font-weight="bold">world</tspan></text></svg>"##;
        let stacked = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200"><text x="10" y="40" font-size="20">Hello <tspan x="10" y="40">world</tspan></text></svg>"##;

        let width = |svg: &str| {
            let doc = parse_svg(svg).unwrap();
            let info = doc
                .tree
                .as_ref()
                .unwrap()
                .text_infos()
                .first()
                .cloned()
                .unwrap();
            let d = outline_runs_d(&font, 0, &info.layout(), 3).unwrap();
            let b = crate::model::geometry::subpaths_bounds(&parse_path_d(&d)).unwrap();
            b.max_x - b.min_x
        };
        assert!(
            width(one_line) > width(stacked) * 1.5,
            "continuing runs on: {} vs restarting at the same x: {}",
            width(one_line),
            width(stacked)
        );
    }

    /// A tspan may ask for a family of its own. One font shapes the whole label, so the caller is
    /// told which families it is about to override rather than finding out from the result.
    #[test]
    fn foreign_families_are_reported_not_silently_shaped() {
        use crate::model::document::parse_svg;
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200"><text font-family="Inter">plain <tspan font-family="Courier New">code</tspan></text></svg>"##;
        let doc = parse_svg(svg).unwrap();
        let info = doc
            .tree
            .as_ref()
            .unwrap()
            .text_infos()
            .first()
            .cloned()
            .unwrap();
        assert_eq!(info.family, "Inter", "the label's own family");
        assert_eq!(info.foreign_families(), vec!["Courier New".to_string()]);

        // A label whose runs all agree has nothing to report.
        let same = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200"><text font-family="Inter">plain <tspan x="0">more</tspan></text></svg>"##;
        let doc = parse_svg(same).unwrap();
        let info = doc
            .tree
            .as_ref()
            .unwrap()
            .text_infos()
            .first()
            .cloned()
            .unwrap();
        assert!(info.foreign_families().is_empty());
    }
}
