//! SVG document parse + serialize — ported from `model/document.ts`.
//!
//! Parsing reads structure with `roxmltree` (mirroring the TS `DOMParser` + `querySelectorAll`)
//! and captures each opening `<path …>` tag verbatim for surgical rewrites. Serializing
//! preserves the source byte-for-byte except the `d`/`id`/style of paths the user edited,
//! which are spliced in place; in-app-drawn paths are appended before `</svg>`.

use indexmap::IndexMap;
use kurbo::{BezPath, Shape};

use super::path::{parse_path_d, path_to_d_prec};
use super::types::{PathElement, SvgDocument, ViewBox};

const DEFAULT_VIEWBOX: ViewBox = ViewBox {
    min_x: 0.0,
    min_y: 0.0,
    width: 100.0,
    height: 100.0,
};

/// Presentation attributes the STYLE panel can read/edit on any path. Parsed from imported
/// paths so they can be styled + reset.
pub const STYLE_KEYS: [&str; 9] = [
    "fill",
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
            }
        })
        .collect();

    let view_box = read_view_box(&root, &paths);
    Ok(SvgDocument {
        source: source.to_string(),
        view_box,
        paths,
    })
}

fn style_overridden(p: &PathElement) -> bool {
    p.style_override.as_ref().is_some_and(|m| !m.is_empty())
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

/// Insert in-app-drawn paths (no source location) just before the closing `</svg>`.
fn append_drawn_paths(out: &str, doc: &SvgDocument, precision: usize) -> String {
    let drawn: Vec<String> = doc
        .paths
        .iter()
        .filter(|p| p.added && !p.deleted && p.subpaths.iter().any(|sp| sp.nodes.len() >= 2))
        .map(|p| {
            let id = if p.renamed {
                format!(" id=\"{}\"", p.id)
            } else {
                String::new()
            };
            format!(
                "  <path{} d=\"{}\"{} />",
                id,
                path_to_d_prec(&p.subpaths, precision),
                attrs_to_string(p.attributes.as_ref())
            )
        })
        .collect();
    let drawn = drawn.join("\n");
    if drawn.is_empty() {
        return out.to_string();
    }
    match out.rfind("</svg>") {
        Some(close) => format!("{}{}\n{}", &out[..close], drawn, &out[close..]),
        None => format!("{out}\n{drawn}"),
    }
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
    for p in &doc.paths {
        if p.added {
            continue;
        }
        let Some(tag) = &p.original_tag else {
            continue;
        };
        let Some(rel) = src[cursor..].find(tag.as_str()) else {
            continue;
        };
        let idx = cursor + rel;
        let end = idx + tag.len();
        if p.deleted {
            out.push_str(&src[cursor..idx]); // drop the tag
        } else if p.edited || p.renamed || style_overridden(p) {
            let mut t = tag.clone();
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
            out.push_str(&src[cursor..idx]);
            out.push_str(&t);
        } else {
            out.push_str(&src[cursor..end]); // verbatim
        }
        cursor = end;
    }
    out.push_str(&src[cursor..]);
    append_drawn_paths(&out, doc, precision)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::types::Point;

    const SAMPLE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <rect x="0" y="0" width="100" height="100" fill="#eee"/>
  <path id="a" d="M 10 10 L 90 10 L 90 90" fill="none" stroke="black"/>
  <path d="M 20 20 L 80 80" stroke="red"/>
</svg>"##;

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
    fn maps_duplicate_d_values_to_the_right_element() {
        let dup = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M0 0 L5 5"/><path d="M0 0 L5 5"/></svg>"#;
        let mut doc = parse_svg(dup).unwrap();
        doc.paths[1].subpaths[0].nodes[1].point = Point::new(9.0, 9.0);
        doc.paths[1].edited = true;
        let out = serialize_svg(&doc);
        assert!(out.contains(r#"<path d="M0 0 L5 5"/><path d="M 0 0 L 9 9"/>"#));
    }
}
