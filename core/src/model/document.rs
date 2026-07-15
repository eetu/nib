//! SVG document parse + serialize — ported from `model/document.ts`.
//!
//! Parsing reads structure with `roxmltree` (mirroring the TS `DOMParser` + `querySelectorAll`)
//! and captures each opening `<path …>` tag verbatim for surgical rewrites. Serializing
//! preserves the source byte-for-byte except the `d`/`id`/style of paths the user edited,
//! which are spliced in place; in-app-drawn paths are appended before `</svg>`.

use indexmap::IndexMap;
use kurbo::{BezPath, Shape};

use super::path::{parse_path_d, path_to_d_prec};
use super::tree::{Tree, serialize_tree};
use super::types::{Gradient, Layer, PathElement, Subpath, SvgDocument, ViewBox};

const DEFAULT_VIEWBOX: ViewBox = ViewBox {
    min_x: 0.0,
    min_y: 0.0,
    width: 100.0,
    height: 100.0,
};

/// Presentation attributes the STYLE panel can read/edit on any path. Parsed from imported
/// paths so they can be styled + reset.
pub const STYLE_KEYS: [&str; 10] = [
    "fill",
    "fill-rule",
    "stroke",
    "stroke-width",
    "opacity",
    "fill-opacity",
    "stroke-opacity",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-dasharray",
];

fn parse_style_attrs(el: &roxmltree::Node) -> IndexMap<String, String> {
    let mut attrs = IndexMap::new();
    for key in STYLE_KEYS {
        if let Some(v) = el.attribute(key) {
            attrs.insert(key.to_string(), v.to_string());
        }
    }
    attrs
}

/// Parse the leading numeric prefix of a string, mirroring JS `parseFloat` (so "40px" → 40).
fn parse_leading_f64(s: &str) -> Option<f64> {
    let t = s.trim_start();
    let bytes = t.as_bytes();
    let mut end = 0;
    while end < bytes.len() {
        let c = bytes[end];
        if c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.' | b'e' | b'E') {
            end += 1;
        } else {
            break;
        }
    }
    t[..end].parse::<f64>().ok()
}

fn read_view_box(svg: &roxmltree::Node, paths: &[PathElement]) -> ViewBox {
    if let Some(vb) = svg.attribute("viewBox") {
        let nums: Vec<f64> = vb
            .trim()
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<f64>().unwrap_or(f64::NAN))
            .collect();
        if nums.len() == 4 && nums.iter().all(|n| n.is_finite()) && nums[2] > 0.0 && nums[3] > 0.0 {
            return ViewBox {
                min_x: nums[0],
                min_y: nums[1],
                width: nums[2],
                height: nums[3],
            };
        }
    }
    let w = svg.attribute("width").and_then(parse_leading_f64);
    let h = svg.attribute("height").and_then(parse_leading_f64);
    if let (Some(w), Some(h)) = (w, h)
        && w.is_finite()
        && h.is_finite()
        && w > 0.0
        && h > 0.0
    {
        return ViewBox {
            min_x: 0.0,
            min_y: 0.0,
            width: w,
            height: h,
        };
    }
    bounds_of(paths).unwrap_or(DEFAULT_VIEWBOX)
}

/// Fallback viewBox from the union of all path bounds, padded 5%.
fn bounds_of(paths: &[PathElement]) -> Option<ViewBox> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for p in paths {
        if p.original_d.is_empty() {
            continue;
        }
        if let Ok(bez) = BezPath::from_svg(&p.original_d) {
            let b = bez.bounding_box();
            min_x = min_x.min(b.x0);
            min_y = min_y.min(b.y0);
            max_x = max_x.max(b.x1);
            max_y = max_y.max(b.y1);
        }
    }
    if !min_x.is_finite() || max_x <= min_x || max_y <= min_y {
        return None;
    }
    let pad_x = (max_x - min_x) * 0.05;
    let pad_y = (max_y - min_y) * 0.05;
    Some(ViewBox {
        min_x: min_x - pad_x,
        min_y: min_y - pad_y,
        width: max_x - min_x + pad_x * 2.0,
        height: max_y - min_y + pad_y * 2.0,
    })
}

/// Every opening `<path …>` tag in source order (mirrors the TS `/<path\b[^>]*>/gi`), the
/// verbatim anchors for surgical rewrites — index-aligned with the parsed path elements.
fn extract_path_tags(source: &str) -> Vec<String> {
    let lower = source.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut tags = Vec::new();
    let mut i = 0;
    while let Some(rel) = lower[i..].find("<path") {
        let start = i + rel;
        let after = start + 5;
        // `\b` after "path": next char must not be a word char.
        let boundary = match bytes.get(after) {
            None => true,
            Some(&c) => !(c.is_ascii_alphanumeric() || c == b'_'),
        };
        if !boundary {
            i = after;
            continue;
        }
        match source[start..].find('>') {
            Some(gt_rel) => {
                let end = start + gt_rel + 1;
                tags.push(source[start..end].to_string());
                i = end;
            }
            None => break,
        }
    }
    tags
}

/// Parse an SVG source string into the editable document model. Errors on markup with no
/// `<svg>` root or that fails to parse.
pub fn parse_svg(source: &str) -> Result<SvgDocument, String> {
    let doc =
        roxmltree::Document::parse(source).map_err(|e| format!("could not parse SVG: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg" {
        return Err("no <svg> root element found".to_string());
    }

    let tags = extract_path_tags(source);
    let path_nodes: Vec<roxmltree::Node> = root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "path")
        .collect();

    let paths: Vec<PathElement> = path_nodes
        .iter()
        .enumerate()
        .map(|(index, el)| {
            let original_d = el.attribute("d").unwrap_or("").to_string();
            let id = el
                .attribute("id")
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("path-{index}"));
            PathElement {
                id,
                uid: String::new(),
                index,
                subpaths: parse_path_d(&original_d),
                attributes: Some(parse_style_attrs(el)),
                original_tag: tags.get(index).cloned(),
                original_d,
                edited: false,
                added: false,
                style_override: None,
                deleted: false,
                renamed: false,
                layer: None,
                hidden: false,
            }
        })
        .collect();

    let view_box = read_view_box(&root, &paths);
    Ok(SvgDocument {
        source: source.to_string(),
        view_box,
        paths,
        layers: Vec::new(),
        active_layer: None,
        gradients: Vec::new(),
        // The structural model — parsed from the same source (never fails if the doc parsed).
        tree: super::tree::parse_tree(source).ok(),
    })
}

/// An imported path's opening tag with its edits applied (d / id / style-override / hidden).
/// With no edits it returns the original tag verbatim, so unchanged paths stay byte-for-byte.
fn edited_tag(p: &PathElement, precision: usize, hidden: bool) -> String {
    let mut t = p.original_tag.clone().unwrap_or_default();
    if p.edited {
        t = with_attr(&t, "d", &path_to_d_prec(&p.subpaths, precision));
    }
    if p.renamed {
        t = with_attr(&t, "id", &p.id);
    }
    if let Some(so) = &p.style_override {
        for (k, v) in so {
            t = with_attr(&t, k, v);
        }
    }
    if hidden {
        t = with_attr(&t, "display", "none");
    }
    t
}

/// Set (or insert) one attribute in a `<path …>` tag string, preserving the rest of the tag
/// byte-for-byte. Replaces the quoted value if the key is present, else inserts the attr
/// just before the closing `>`/`/>`.
fn with_attr(tag: &str, key: &str, value: &str) -> String {
    if let Some((open, close_after, quote)) = find_attr_value_span(tag, key) {
        let mut out = String::with_capacity(tag.len() + value.len());
        out.push_str(&tag[..open]);
        out.push(quote);
        out.push_str(value);
        out.push(quote);
        out.push_str(&tag[close_after..]);
        return out;
    }
    insert_attr(tag, key, value)
}

/// Find the span of a `key="…"` (or `key='…'`) attribute value including its quotes:
/// returns (index of opening quote, index just past the closing quote, quote char). The key
/// must be preceded by whitespace and followed by optional-ws `=` optional-ws, so it never
/// matches a substring of another attribute (e.g. `stroke` inside `stroke-width`).
fn find_attr_value_span(tag: &str, key: &str) -> Option<(usize, usize, char)> {
    let bytes = tag.as_bytes();
    let mut from = 0;
    while let Some(rel) = tag[from..].find(key) {
        let kstart = from + rel;
        let kend = kstart + key.len();
        from = kend;
        if kstart == 0 || !bytes[kstart - 1].is_ascii_whitespace() {
            continue;
        }
        let mut j = kend;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'=' {
            continue;
        }
        j += 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let quote = match bytes.get(j) {
            Some(&b'"') => '"',
            Some(&b'\'') => '\'',
            _ => continue,
        };
        let open = j;
        if let Some(crel) = tag[open + 1..].find(quote) {
            let close = open + 1 + crel;
            return Some((open, close + 1, quote));
        }
    }
    None
}

/// Insert ` key="value"` immediately before the tag's closing `[ws][/]?>` (mirrors the TS
/// `tag.replace(/\s*\/?>$/, …)`).
fn insert_attr(tag: &str, key: &str, value: &str) -> String {
    if !tag.ends_with('>') {
        return format!("{tag} {key}=\"{value}\"");
    }
    let bytes = tag.as_bytes();
    let mut close_start = tag.len() - 1; // the '>'
    if close_start > 0 && bytes[close_start - 1] == b'/' {
        close_start -= 1;
    }
    while close_start > 0 && bytes[close_start - 1].is_ascii_whitespace() {
        close_start -= 1;
    }
    let mut out = String::with_capacity(tag.len() + key.len() + value.len() + 4);
    out.push_str(&tag[..close_start]);
    out.push_str(&format!(" {key}=\"{value}\""));
    out.push_str(&tag[close_start..]);
    out
}

fn attrs_to_string(attrs: Option<&IndexMap<String, String>>) -> String {
    match attrs {
        None => String::new(),
        Some(m) => m.iter().map(|(k, v)| format!(" {k}=\"{v}\"")).collect(),
    }
}

/// A drawn path is exportable if it has real geometry.
fn is_exportable_drawn(p: &PathElement) -> bool {
    p.added && !p.deleted && p.subpaths.iter().any(|sp| sp.nodes.len() >= 2)
}

/// A path element's effective style — its `attributes` with any `style_override` merged over.
pub fn effective_style(p: &PathElement) -> IndexMap<String, String> {
    let mut m = p.attributes.clone().unwrap_or_default();
    if let Some(so) = &p.style_override {
        for (k, v) in so {
            m.insert(k.clone(), v.clone());
        }
    }
    m
}

/// The subject of a boolean group = the backmost member that actually fills (falls back to the
/// backmost member), whose effective style the computed result inherits.
fn boolean_subject<'a>(members: &[&'a PathElement]) -> Option<&'a PathElement> {
    members
        .iter()
        .find(|p| {
            let fill = p
                .style_override
                .as_ref()
                .and_then(|s| s.get("fill"))
                .or_else(|| p.attributes.as_ref().and_then(|a| a.get("fill")));
            match fill {
                Some(f) => f.as_str() != "none",
                None => true,
            }
        })
        .or_else(|| members.first())
        .copied()
}

/// Live members (operands) of a boolean group, in draw (array) order — excludes deleted paths.
pub fn boolean_members<'a>(doc: &'a SvgDocument, layer_id: &str) -> Vec<&'a PathElement> {
    doc.paths
        .iter()
        .filter(|p| !p.deleted && p.layer.as_deref() == Some(layer_id))
        .collect()
}

/// Compute a live boolean group's result: `(computed subpaths, subject effective style)`.
/// `None` if the layer isn't a boolean group or the boolean can't be formed (< 2 operands /
/// empty geometry) — callers then fall back to rendering/exporting the members directly.
pub fn boolean_group_result(
    doc: &SvgDocument,
    layer: &Layer,
) -> Option<(Vec<Subpath>, IndexMap<String, String>)> {
    let op = layer.boolean_op.as_deref()?;
    let members = boolean_members(doc, &layer.id);
    let subpaths = crate::model::booleans::boolean(op, &members)?;
    let style = boolean_subject(&members)
        .map(effective_style)
        .unwrap_or_default();
    Some((subpaths, style))
}

/// Serialize one drawn `<path …>` (indented). `indent` is the leading whitespace.
fn drawn_path_tag(p: &PathElement, precision: usize, indent: &str) -> String {
    let id = if p.renamed {
        format!(" id=\"{}\"", p.id)
    } else {
        String::new()
    };
    let hidden = if p.hidden { " display=\"none\"" } else { "" };
    format!(
        "{}<path{}{} d=\"{}\"{} />",
        indent,
        id,
        hidden,
        path_to_d_prec(&p.subpaths, precision),
        attrs_to_string(p.attributes.as_ref())
    )
}

/// Escape a string for use inside a double-quoted XML attribute value.
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}

/// Insert in-app-drawn paths (no source location) just before the closing `</svg>`, in draw
/// (array) order — so the PATHS/layers panel z-order matches the export. A contiguous run of
/// paths sharing a group id is wrapped in `<g id="name">` (a hidden group's `<g>` carries
/// `display="none"`); loose paths emit bare. Members are kept contiguous by GroupPaths.
fn append_drawn_paths(out: &str, doc: &SvgDocument, precision: usize) -> String {
    let drawn: Vec<&PathElement> = doc
        .paths
        .iter()
        .filter(|p| is_exportable_drawn(p))
        .collect();
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < drawn.len() {
        let group = drawn[i]
            .layer
            .as_deref()
            .and_then(|id| doc.layers.iter().find(|l| l.id == id));
        if let Some(layer) = group {
            let start = i;
            while i < drawn.len() && drawn[i].layer.as_deref() == Some(layer.id.as_str()) {
                i += 1;
            }
            let hidden = if layer.visible {
                ""
            } else {
                " display=\"none\""
            };
            // A live boolean group bakes to ONE computed <path> (operands aren't emitted) —
            // SVG can't express the live op, so export renders correctly everywhere; the
            // liveness lives in nib's model. A plain group emits its members.
            let inner = match boolean_group_result(doc, layer) {
                Some((subpaths, style)) => {
                    let attrs: String =
                        style.iter().map(|(k, v)| format!(" {k}=\"{v}\"")).collect();
                    format!(
                        "    <path d=\"{}\"{} />",
                        path_to_d_prec(&subpaths, precision),
                        attrs
                    )
                }
                None => drawn[start..i]
                    .iter()
                    .map(|p| drawn_path_tag(p, precision, "    "))
                    .collect::<Vec<_>>()
                    .join("\n"),
            };
            parts.push(format!(
                "  <g id=\"{}\"{}>\n{}\n  </g>",
                escape_attr(&layer.name),
                hidden,
                inner
            ));
        } else {
            parts.push(drawn_path_tag(drawn[i], precision, "  "));
            i += 1;
        }
    }
    let block = parts.join("\n");
    if block.is_empty() {
        return out.to_string();
    }
    match out.rfind("</svg>") {
        Some(close) => format!("{}{}\n{}", &out[..close], block, &out[close..]),
        None => format!("{out}\n{block}"),
    }
}

/// Whether an imported path sits on a hidden layer (so it exports with `display="none"`).
fn on_hidden_layer(doc: &SvgDocument, p: &PathElement) -> bool {
    p.layer
        .as_deref()
        .and_then(|id| doc.layers.iter().find(|l| l.id == id))
        .is_some_and(|l| !l.visible)
}

/// A path is hidden if toggled off directly or its group is hidden.
fn path_hidden(doc: &SvgDocument, p: &PathElement) -> bool {
    p.hidden || on_hidden_layer(doc, p)
}

/// One gradient as a `<linearGradient>` / `<radialGradient>` element.
fn gradient_to_svg(g: &Gradient) -> String {
    let stops: String = g
        .stops
        .iter()
        .map(|s| {
            let op = s
                .opacity
                .map(|o| format!(" stop-opacity=\"{o}\""))
                .unwrap_or_default();
            format!(
                "      <stop offset=\"{}\" stop-color=\"{}\"{} />",
                s.offset,
                escape_attr(&s.color),
                op
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    if g.kind == "radial" {
        format!(
            "    <radialGradient id=\"{}\" cx=\"{}\" cy=\"{}\" r=\"{}\">\n{}\n    </radialGradient>",
            escape_attr(&g.id),
            g.cx,
            g.cy,
            g.r,
            stops
        )
    } else {
        format!(
            "    <linearGradient id=\"{}\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\">\n{}\n    </linearGradient>",
            escape_attr(&g.id),
            g.x1,
            g.y1,
            g.x2,
            g.y2,
            stops
        )
    }
}

/// Inject nib's gradient paints as a `<defs>` right after the `<svg …>` open tag (a
/// head-injection step, parallel to appending drawn paths — the source is otherwise
/// untouched). No-op when there are no gradients.
fn inject_defs(out: &str, doc: &SvgDocument) -> String {
    if doc.gradients.is_empty() {
        return out.to_string();
    }
    let body = doc
        .gradients
        .iter()
        .map(gradient_to_svg)
        .collect::<Vec<_>>()
        .join("\n");
    let defs = format!("  <defs>\n{body}\n  </defs>");
    let lower = out.to_ascii_lowercase();
    match lower
        .find("<svg")
        .and_then(|s| out[s..].find('>').map(|g| s + g + 1))
    {
        Some(pos) => format!("{}\n{}{}", &out[..pos], defs, &out[pos..]),
        None => out.to_string(),
    }
}

/// Union bounds (nodes + handles) of every non-deleted path, or None if empty.
fn union_content_bounds(doc: &SvgDocument) -> Option<(f64, f64, f64, f64)> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut any = false;
    for p in &doc.paths {
        if p.deleted {
            continue;
        }
        for sp in &p.subpaths {
            for n in &sp.nodes {
                for pt in [Some(n.point), n.handle_in, n.handle_out]
                    .into_iter()
                    .flatten()
                {
                    any = true;
                    min_x = min_x.min(pt.x);
                    min_y = min_y.min(pt.y);
                    max_x = max_x.max(pt.x);
                    max_y = max_y.max(pt.y);
                }
            }
        }
    }
    (any && max_x > min_x && max_y > min_y).then_some((min_x, min_y, max_x, max_y))
}

/// The viewBox to export: the declared artboard, grown to include any content drawn outside
/// it — so a shape placed beyond the source viewBox isn't clipped when the file is reopened
/// elsewhere. Equals the source viewBox when all content fits (→ no rewrite).
fn export_view_box(doc: &SvgDocument) -> ViewBox {
    let vb = doc.view_box;
    match union_content_bounds(doc) {
        Some((x0, y0, x1, y1)) => {
            let min_x = vb.min_x.min(x0);
            let min_y = vb.min_y.min(y0);
            ViewBox {
                min_x,
                min_y,
                width: (vb.min_x + vb.width).max(x1) - min_x,
                height: (vb.min_y + vb.height).max(y1) - min_y,
            }
        }
        None => vb,
    }
}

fn round3(n: f64) -> f64 {
    (n * 1000.0).round() / 1000.0
}

/// Rewrite the `<svg …>` opening tag's `viewBox` attribute (used only when content overflows).
fn rewrite_svg_viewbox(out: &str, vb: ViewBox) -> String {
    let lower = out.to_ascii_lowercase();
    let Some(start) = lower.find("<svg") else {
        return out.to_string();
    };
    let Some(gt) = out[start..].find('>').map(|g| start + g + 1) else {
        return out.to_string();
    };
    let value = format!(
        "{} {} {} {}",
        round3(vb.min_x),
        round3(vb.min_y),
        round3(vb.width),
        round3(vb.height)
    );
    let new_tag = with_attr(&out[start..gt], "viewBox", &value);
    format!("{}{}{}", &out[..start], new_tag, &out[gt..])
}

/// Serialize the document to SVG at the TS default precision (3).
pub fn serialize_svg(doc: &SvgDocument) -> String {
    serialize_svg_prec(doc, 3)
}

/// Serialize the document to SVG. Everything is preserved byte-for-byte except the `d`/`id`/
/// style of edited paths, spliced in place; paths are located in document order via a moving
/// cursor so duplicate `d` values still map to the right element.
pub fn serialize_svg_prec(doc: &SvgDocument, precision: usize) -> String {
    let src = &doc.source;
    let mut out = String::new();
    let mut cursor = 0;
    // The `<path>` slots in the source, in source order, locate the byte spans (found via a
    // moving cursor, so duplicate tags still map right). We *fill* each non-deleted slot with
    // the next imported path in **draw order** (array order) — so reordering paths reorders
    // the exported z-order while non-path content + slot positions stay byte-for-byte. With no
    // reordering, each slot gets its own path back → byte-for-byte.
    let ordered: Vec<&PathElement> = doc
        .paths
        .iter()
        .filter(|p| !p.added && !p.deleted)
        .collect();
    let mut slots: Vec<&PathElement> = doc.paths.iter().filter(|p| !p.added).collect();
    slots.sort_by_key(|p| p.index);
    let mut oi = 0;
    for slot in slots {
        let Some(tag) = &slot.original_tag else {
            continue;
        };
        let Some(rel) = src[cursor..].find(tag.as_str()) else {
            continue;
        };
        let idx = cursor + rel;
        let end = idx + tag.len();
        out.push_str(&src[cursor..idx]);
        if !slot.deleted && oi < ordered.len() {
            let p = ordered[oi];
            oi += 1;
            out.push_str(&edited_tag(p, precision, path_hidden(doc, p)));
        }
        // a deleted slot drops its tag (emits only the preceding span)
        cursor = end;
    }
    out.push_str(&src[cursor..]);
    let with_drawn = append_drawn_paths(&out, doc, precision);
    let with_defs = inject_defs(&with_drawn, doc);
    let evb = export_view_box(doc);
    if evb != doc.view_box {
        rewrite_svg_viewbox(&with_defs, evb)
    } else {
        with_defs
    }
}

/// Serialize through the **document tree** (Phase E) rather than the flat splice: reconcile the
/// flat paths' edits onto a clone of the parsed `base` tree, emit it (byte-for-byte for untouched
/// nodes; edited primitives become `<path>`), then append drawn paths + inject gradient defs +
/// grow the viewBox exactly as the splice path does. This is what makes editing *non-path*
/// elements exportable — the tree carries the full structure the flat splice can't.
pub fn serialize_via_tree(doc: &SvgDocument, base: &Tree, precision: usize) -> String {
    let mut tree = base.clone();
    tree.reconcile_paths(&doc.paths, precision);
    let out = serialize_tree(&tree);
    let with_drawn = append_drawn_paths(&out, doc, precision);
    let with_defs = inject_defs(&with_drawn, doc);
    let evb = export_view_box(doc);
    if evb != doc.view_box {
        rewrite_svg_viewbox(&with_defs, evb)
    } else {
        with_defs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::path::parse_path_d;
    use crate::model::types::{Layer, Point};

    fn drawn_on_layer(id: &str, layer: &str) -> PathElement {
        PathElement {
            id: id.to_string(),
            uid: String::new(),
            index: 0,
            original_d: String::new(),
            subpaths: parse_path_d("M 0 0 L 10 10"),
            edited: true,
            added: true,
            attributes: None,
            style_override: None,
            original_tag: None,
            deleted: false,
            renamed: false,
            layer: Some(layer.to_string()),
            hidden: false,
        }
    }

    const SAMPLE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <rect x="0" y="0" width="100" height="100" fill="#eee"/>
  <path id="a" d="M 10 10 L 90 10 L 90 90" fill="none" stroke="black"/>
  <path d="M 20 20 L 80 80" stroke="red"/>
</svg>"##;

    #[test]
    fn via_tree_keeps_unedited_primitives_verbatim() {
        // The Editor's export path (serialize_via_tree) on a doc whose paths are projected from
        // the tree but *unedited* must stay byte-for-byte — primitives keep their `<rect>` etc.,
        // only an edited primitive converts to `<path>`.
        let src = include_str!("../../tests/fixtures/shapes.svg");
        let tree = crate::model::tree::parse_tree(src).unwrap();
        let mut doc = parse_svg(src).unwrap();
        doc.paths = tree.project_paths(); // what the Editor does on load
        assert_eq!(serialize_via_tree(&doc, &tree, 3), src);

        // Editing one primitive converts only it; the rest stay verbatim.
        doc.paths[0].subpaths[0].nodes[0].point = Point::new(21.0, 21.0);
        doc.paths[0].edited = true;
        let out = serialize_via_tree(&doc, &tree, 3);
        assert!(!out.contains("<rect"), "edited rect → path");
        assert!(out.contains("<circle"), "unedited circle stays <circle>");
        assert!(out.contains("<ellipse") && out.contains("<polygon"));
    }

    #[test]
    fn reads_viewbox_and_enumerates_paths_in_order() {
        let doc = parse_svg(SAMPLE).unwrap();
        assert_eq!(
            doc.view_box,
            ViewBox {
                min_x: 0.0,
                min_y: 0.0,
                width: 100.0,
                height: 100.0
            }
        );
        assert_eq!(doc.paths.len(), 2);
        assert_eq!(doc.paths[0].id, "a");
        assert_eq!(doc.paths[1].id, "path-1");
        assert_eq!(doc.paths[0].index, 0);
    }

    #[test]
    fn synthesizes_viewbox_from_width_height() {
        let doc = parse_svg(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30"><path d="M0 0 L1 1"/></svg>"#,
        )
        .unwrap();
        assert_eq!(
            doc.view_box,
            ViewBox {
                min_x: 0.0,
                min_y: 0.0,
                width: 40.0,
                height: 30.0
            }
        );
    }

    #[test]
    fn errors_on_markup_with_no_svg_root() {
        assert!(parse_svg("<div>not svg</div>").is_err());
    }

    #[test]
    fn preserves_source_byte_for_byte_when_nothing_edited() {
        let doc = parse_svg(SAMPLE).unwrap();
        assert_eq!(serialize_svg(&doc), SAMPLE);
    }

    #[test]
    fn parses_imported_path_style_attributes() {
        let doc = parse_svg(SAMPLE).unwrap();
        let attrs = doc.paths[0].attributes.as_ref().unwrap();
        assert_eq!(attrs.get("fill").map(String::as_str), Some("none"));
        assert_eq!(attrs.get("stroke").map(String::as_str), Some("black"));
    }

    #[test]
    fn applies_a_style_override_preserving_the_rest_of_the_tag() {
        let mut doc = parse_svg(SAMPLE).unwrap();
        let mut ov = IndexMap::new();
        ov.insert("stroke".to_string(), "red".to_string());
        ov.insert("stroke-width".to_string(), "3".to_string());
        doc.paths[0].style_override = Some(ov);
        let out = serialize_svg(&doc);
        assert!(out.contains("stroke=\"red\""));
        assert!(!out.contains("stroke=\"black\""));
        assert!(out.contains("stroke-width=\"3\""));
        assert!(out.contains("id=\"a\""));
        assert!(out.contains("fill=\"none\""));
        assert!(out.contains("d=\"M 10 10 L 90 10 L 90 90\""));
        assert!(out.contains(r#"<path d="M 20 20 L 80 80" stroke="red"/>"#));
    }

    #[test]
    fn splices_only_the_edited_paths_d() {
        let mut doc = parse_svg(SAMPLE).unwrap();
        doc.paths[1].subpaths[0].nodes[1].point = Point::new(70.0, 70.0);
        doc.paths[1].edited = true;
        let out = serialize_svg(&doc);
        assert!(out.contains(r##"<rect x="0" y="0" width="100" height="100" fill="#eee"/>"##));
        assert!(out.contains(r#"d="M 10 10 L 90 10 L 90 90""#));
        assert!(out.contains(r#"d="M 20 20 L 70 70""#));
        assert!(!out.contains(r#"d="M 20 20 L 80 80""#));
    }

    #[test]
    fn drawn_paths_group_into_layer_g_elements_and_honor_visibility() {
        let mut doc =
            parse_svg("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\">\n</svg>")
                .unwrap();
        doc.layers.push(Layer {
            id: "L1".into(),
            name: "outline".into(),
            visible: true,
            boolean_op: None,
        });
        doc.paths.push(drawn_on_layer("s1", "L1"));
        let out = serialize_svg(&doc);
        assert!(out.contains(r#"<g id="outline">"#), "grouped: {out}");
        assert!(out.contains("<path"));
        assert!(out.contains("</g>"));

        // Hiding the layer stamps display:none on its group.
        doc.layers[0].visible = false;
        let out2 = serialize_svg(&doc);
        assert!(
            out2.contains(r#"<g id="outline" display="none">"#),
            "hidden: {out2}"
        );
    }

    #[test]
    fn live_boolean_group_bakes_to_one_computed_path_on_export() {
        let mut doc =
            parse_svg("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\">\n</svg>")
                .unwrap();
        doc.layers.push(Layer {
            id: "B1".into(),
            name: "cut".into(),
            visible: true,
            boolean_op: Some("subtract".into()),
        });
        // Two overlapping filled squares as operands (subject minus cutter).
        let mut subject = drawn_on_layer("subject", "B1");
        subject.subpaths = parse_path_d("M 0 0 L 60 0 L 60 60 L 0 60 Z");
        let mut fill = IndexMap::new();
        fill.insert("fill".to_string(), "#3b82f6".to_string());
        subject.attributes = Some(fill);
        let mut cutter = drawn_on_layer("cutter", "B1");
        cutter.subpaths = parse_path_d("M 40 40 L 100 40 L 100 100 L 40 100 Z");
        doc.paths.push(subject);
        doc.paths.push(cutter);

        let out = serialize_svg(&doc);
        assert!(out.contains(r#"<g id="cut">"#), "grouped: {out}");
        // Exactly one baked path inside the group (operands are not emitted), carrying the
        // subject's fill.
        assert_eq!(out.matches("<path").count(), 1, "one baked path: {out}");
        assert!(
            out.contains(r##"fill="#3b82f6""##),
            "subject fill kept: {out}"
        );

        // The computed result exists (subtract of two 60×60 squares overlapping in a corner).
        let (subpaths, _) = boolean_group_result(&doc, &doc.layers[0]).expect("result");
        assert!(!subpaths.is_empty());
    }

    #[test]
    fn boolean_group_with_one_operand_falls_back_to_emitting_members() {
        let mut doc =
            parse_svg("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\">\n</svg>")
                .unwrap();
        doc.layers.push(Layer {
            id: "B1".into(),
            name: "cut".into(),
            visible: true,
            boolean_op: Some("subtract".into()),
        });
        doc.paths.push(drawn_on_layer("only", "B1")); // < 2 operands → no boolean
        assert!(boolean_group_result(&doc, &doc.layers[0]).is_none());
        let out = serialize_svg(&doc);
        assert!(out.contains(r#"<g id="cut">"#), "still a group: {out}");
        assert_eq!(out.matches("<path").count(), 1); // the lone member emitted directly
    }

    #[test]
    fn imported_path_on_a_hidden_layer_gets_display_none() {
        let mut doc = parse_svg(SAMPLE).unwrap();
        doc.layers.push(Layer {
            id: "L1".into(),
            name: "hidden".into(),
            visible: false,
            boolean_op: None,
        });
        doc.paths[0].layer = Some("L1".into());
        let out = serialize_svg(&doc);
        assert!(out.contains("display=\"none\""), "{out}");
        // The other path (no layer) is untouched.
        assert!(out.contains(r#"<path d="M 20 20 L 80 80" stroke="red"/>"#));
    }

    #[test]
    fn reordering_imported_paths_swaps_their_export_positions() {
        let mut doc = parse_svg(SAMPLE).unwrap();
        // No reorder → byte-for-byte.
        assert_eq!(serialize_svg(&doc), SAMPLE);
        // Swap draw order → the two <path> tags swap document positions (z-order follows the
        // array); non-path content (the rect) stays byte-for-byte in place.
        doc.paths.swap(0, 1);
        let out = serialize_svg(&doc);
        let red = out
            .find(r#"<path d="M 20 20 L 80 80" stroke="red"/>"#)
            .unwrap();
        let black = out.find(r#"d="M 10 10 L 90 10 L 90 90""#).unwrap();
        assert!(red < black, "draw order should follow the array: {out}");
        assert!(out.contains(r##"<rect x="0" y="0" width="100" height="100" fill="#eee"/>"##));
    }

    #[test]
    fn export_grows_viewbox_to_cover_overflowing_content() {
        let mut doc =
            parse_svg("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\">\n</svg>")
                .unwrap();
        let mut p = drawn_on_layer("s1", "x");
        p.layer = None;
        p.subpaths = parse_path_d("M 120 120 L 180 120 L 180 180 Z");
        doc.paths.push(p);
        let out = serialize_svg(&doc);
        assert!(out.contains(r#"viewBox="0 0 180 180""#), "{out}");
        // content within the viewBox leaves it byte-for-byte
        let within = parse_svg(SAMPLE).unwrap();
        assert!(serialize_svg(&within).contains(r#"viewBox="0 0 100 100""#));
    }

    #[test]
    fn empty_layers_keep_byte_for_byte_preservation() {
        let doc = parse_svg(SAMPLE).unwrap();
        assert!(doc.layers.is_empty());
        assert_eq!(serialize_svg(&doc), SAMPLE);
    }

    #[test]
    fn gradients_inject_a_defs_after_the_svg_open_tag() {
        use crate::model::types::{Gradient, GradientStop};
        let mut doc = parse_svg(SAMPLE).unwrap();
        // No gradients → byte-for-byte still.
        assert_eq!(serialize_svg(&doc), SAMPLE);
        doc.gradients.push(Gradient {
            id: "g1".into(),
            kind: "linear".into(),
            stops: vec![
                GradientStop {
                    offset: 0.0,
                    color: "#f00".into(),
                    opacity: None,
                },
                GradientStop {
                    offset: 1.0,
                    color: "#00f".into(),
                    opacity: Some(0.5),
                },
            ],
            x1: 0.0,
            y1: 0.0,
            x2: 1.0,
            y2: 0.0,
            cx: 0.5,
            cy: 0.5,
            r: 0.5,
        });
        let out = serialize_svg(&doc);
        assert!(out.contains("<defs>"), "{out}");
        assert!(out.contains(r#"<linearGradient id="g1""#), "{out}");
        assert!(out.contains(r##"stop-color="#f00""##));
        assert!(out.contains(r#"stop-opacity="0.5""#));
        // The original content is still present (defs is additive).
        assert!(out.contains(r##"<rect x="0" y="0" width="100" height="100" fill="#eee"/>"##));
    }

    #[test]
    fn maps_duplicate_d_values_to_the_right_element() {
        let dup = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M0 0 L5 5"/><path d="M0 0 L5 5"/></svg>"#;
        let mut doc = parse_svg(dup).unwrap();
        doc.paths[1].subpaths[0].nodes[1].point = Point::new(9.0, 9.0);
        doc.paths[1].edited = true;
        let out = serialize_svg(&doc);
        assert!(out.contains(r#"<path d="M0 0 L5 5"/><path d="M 0 0 L 9 9"/>"#));
    }
}
