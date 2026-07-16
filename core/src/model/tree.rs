//! Phase-E full-document **node tree** — parse an SVG into a tree where *every* node (element,
//! text, comment, PI) carries its verbatim source span, and re-emit by walking the tree.
//!
//! This is the foundation for editing any element, not just `<path>`: the flat paths-only model
//! keeps non-path content in an opaque `source` string, whereas here the whole document is
//! structured. The safety property (Phase E's whole premise) is **per-node dirty tracking**: an
//! unedited element re-emits its `original_open`/`original_close` verbatim, so byte-for-byte
//! preservation generalizes from paths to all elements; only an *edited* node regenerates its
//! tag. Element types nib doesn't model deeply still round-trip as structured-but-opaque nodes.
//!
//! **Wired into the `Editor`:** it holds a parsed `Tree` as the constant serialization base;
//! `project_paths` seeds the working model (so imported primitives are editable) and
//! `reconcile_paths` + `serialize_tree` write edits back on export (`serialize_via_tree`). Ops +
//! undo stay on the flat `doc.paths`. Byte-for-byte holds *by construction*: the source is
//! partitioned into slices along child boundaries, each owned by exactly one node, so
//! concatenating reproduces it; edits regenerate only their own node.

use std::collections::HashMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::document::STYLE_KEYS;
use super::path::{parse_path_d, path_to_d_prec};
use super::shapes::{ellipse_nodes, line_nodes, rect_nodes};
use super::types::{PathElement, PathNode, Point, Subpath};

/// Geometry attributes stripped when an edited primitive is converted to a `<path>` — its shape
/// now lives in the regenerated `d`, so `x`/`cx`/`points`/… would be dead weight.
const GEOMETRY_ATTRS: [&str; 14] = [
    "x", "y", "width", "height", "cx", "cy", "r", "rx", "ry", "x1", "y1", "x2", "y2", "points",
];

/// Element tags nib can turn into editable anchor-node geometry (subpaths). `<path>` parses its
/// `d`; the primitives convert to the same shape builders drawn shapes use. Anything else stays
/// a structured-but-opaque node (E4/E5 promote text/image/use/defs).
pub const SHAPE_TAGS: [&str; 7] = [
    "path", "rect", "circle", "ellipse", "line", "polygon", "polyline",
];

/// Container elements whose *contents* are not directly-editable canvas content — a `<defs>`
/// subtree (gradients, `<clipPath>`, `<mask>`, `<pattern>`, `<marker>`, `<symbol>`) renders only by
/// reference. Their inner shapes must NOT project as editable paths (else a `<circle>` inside a
/// `<clipPath>` becomes a phantom path in the panel + `doc.paths`); they stay opaque + re-emit
/// verbatim. The render walk still emits them so the defs keep working.
pub const DEF_CONTAINERS: [&str; 8] = [
    "defs",
    "clipPath",
    "mask",
    "pattern",
    "marker",
    "symbol",
    "linearGradient",
    "radialGradient",
];

/// Parse a numeric attribute (SVG geometry values are plain numbers, optionally with a unit
/// suffix like `px` — take the leading number, mirroring JS `parseFloat`).
fn num(attrs: &[(String, String)], key: &str, default: f64) -> f64 {
    let Some(s) = attr(attrs, key) else {
        return default;
    };
    let t = s.trim_start();
    let end = t
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E')))
        .unwrap_or(t.len());
    t[..end].parse::<f64>().unwrap_or(default)
}

/// Parse a `points` list ("x,y x,y" / "x y x y") into corner nodes.
fn parse_points(s: &str) -> Vec<PathNode> {
    let nums: Vec<f64> = s
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
        .filter_map(|t| t.parse::<f64>().ok())
        .collect();
    nums.chunks_exact(2)
        .map(|p| PathNode::corner(Point::new(p[0], p[1])))
        .collect()
}

/// Convert a shape element (tag + attrs) into editable anchor-node subpaths — the bridge that
/// lets imported primitives be edited like drawn shapes. `None` if the tag isn't a shape or its
/// geometry is degenerate/unparseable (→ it stays a structured-but-opaque node, never editable).
pub fn shape_subpaths(tag: &str, attrs: &[(String, String)]) -> Option<Vec<Subpath>> {
    let closed = |nodes: Vec<PathNode>| {
        Some(vec![Subpath {
            nodes,
            closed: true,
        }])
    };
    let open = |nodes: Vec<PathNode>| {
        Some(vec![Subpath {
            nodes,
            closed: false,
        }])
    };
    match tag {
        "path" => {
            let sp = parse_path_d(attr(attrs, "d")?);
            (!sp.is_empty()).then_some(sp)
        }
        "rect" => {
            let (x, y, w, h) = (
                num(attrs, "x", 0.0),
                num(attrs, "y", 0.0),
                num(attrs, "width", 0.0),
                num(attrs, "height", 0.0),
            );
            if w <= 0.0 || h <= 0.0 {
                return None;
            }
            closed(rect_nodes(x, y, x + w, y + h))
        }
        "circle" => {
            let r = num(attrs, "r", 0.0);
            if r <= 0.0 {
                return None;
            }
            closed(ellipse_nodes(
                num(attrs, "cx", 0.0),
                num(attrs, "cy", 0.0),
                r,
                r,
            ))
        }
        "ellipse" => {
            let (rx, ry) = (num(attrs, "rx", 0.0), num(attrs, "ry", 0.0));
            if rx <= 0.0 || ry <= 0.0 {
                return None;
            }
            closed(ellipse_nodes(
                num(attrs, "cx", 0.0),
                num(attrs, "cy", 0.0),
                rx,
                ry,
            ))
        }
        "line" => open(line_nodes(
            num(attrs, "x1", 0.0),
            num(attrs, "y1", 0.0),
            num(attrs, "x2", 0.0),
            num(attrs, "y2", 0.0),
        )),
        "polygon" | "polyline" => {
            let nodes = parse_points(attr(attrs, "points")?);
            if nodes.len() < 2 {
                return None;
            }
            if tag == "polygon" {
                closed(nodes)
            } else {
                open(nodes)
            }
        }
        _ => None,
    }
}

/// Match tolerance (document units) for deciding an edited primitive still fits its form.
const REFIT_EPS: f64 = 1e-3;

fn fnum(v: f64, precision: usize) -> String {
    let s = format!("{v:.precision$}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// Axis-aligned bounds of all node *anchor points* (not bezier extrema — the primitives put
/// their anchors at the extremes, which is what we compare against).
fn points_bbox(subpaths: &[Subpath]) -> Option<(f64, f64, f64, f64)> {
    let mut pts = subpaths
        .iter()
        .flat_map(|s| s.nodes.iter().map(|n| n.point));
    let first = pts.next()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x, first.y);
    for p in pts {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    Some((x0, y0, x1, y1))
}

fn pt_close(a: Point, b: Point) -> bool {
    (a.x - b.x).abs() < REFIT_EPS && (a.y - b.y).abs() < REFIT_EPS
}
fn opt_close(a: Option<Point>, b: Option<Point>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => pt_close(a, b),
        _ => false,
    }
}
/// Do two subpath sets match node-for-node (points + handles) within `REFIT_EPS`?
fn subpaths_match(a: &[Subpath], b: &[Subpath]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(sa, sb)| {
            sa.closed == sb.closed
                && sa.nodes.len() == sb.nodes.len()
                && sa.nodes.iter().zip(&sb.nodes).all(|(na, nb)| {
                    pt_close(na.point, nb.point)
                        && opt_close(na.handle_in, nb.handle_in)
                        && opt_close(na.handle_out, nb.handle_out)
                })
        })
}

fn any_handles(sp: &Subpath) -> bool {
    sp.nodes
        .iter()
        .any(|n| n.handle_in.is_some() || n.handle_out.is_some())
}

/// If the edited geometry *still fits* the primitive `tag`, return the geometry attributes to
/// re-emit it as that primitive (so a moved/resized `<rect>` stays a `<rect>`). `None` means the
/// edit broke the form (e.g. a dragged corner) → the caller falls back to a `<path>`. Detection
/// is by rebuild-and-compare: reconstruct the canonical primitive from the current bounds/points
/// and check it matches, so any form-preserving transform (translate/axis-scale) round-trips.
fn refit(tag: &str, subpaths: &[Subpath], precision: usize) -> Option<Vec<(String, String)>> {
    let n = |v: f64| fnum(v, precision);
    match tag {
        "rect" => {
            let (x0, y0, x1, y1) = points_bbox(subpaths)?;
            let rebuilt = [Subpath {
                nodes: rect_nodes(x0, y0, x1, y1),
                closed: true,
            }];
            subpaths_match(subpaths, &rebuilt).then(|| {
                vec![
                    ("x".into(), n(x0)),
                    ("y".into(), n(y0)),
                    ("width".into(), n(x1 - x0)),
                    ("height".into(), n(y1 - y0)),
                ]
            })
        }
        "circle" | "ellipse" => {
            let (x0, y0, x1, y1) = points_bbox(subpaths)?;
            let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            let (rx, ry) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
            if rx <= 0.0 || ry <= 0.0 {
                return None;
            }
            let rebuilt = [Subpath {
                nodes: ellipse_nodes(cx, cy, rx, ry),
                closed: true,
            }];
            if !subpaths_match(subpaths, &rebuilt) {
                return None;
            }
            if tag == "circle" {
                // Only stays a circle while still round; a non-uniform resize → not a circle.
                ((rx - ry).abs() < REFIT_EPS).then(|| {
                    vec![
                        ("cx".into(), n(cx)),
                        ("cy".into(), n(cy)),
                        ("r".into(), n(rx)),
                    ]
                })
            } else {
                Some(vec![
                    ("cx".into(), n(cx)),
                    ("cy".into(), n(cy)),
                    ("rx".into(), n(rx)),
                    ("ry".into(), n(ry)),
                ])
            }
        }
        "line" => {
            let sp = subpaths.first()?;
            (subpaths.len() == 1 && !sp.closed && sp.nodes.len() == 2 && !any_handles(sp)).then(
                || {
                    let (a, b) = (sp.nodes[0].point, sp.nodes[1].point);
                    vec![
                        ("x1".into(), n(a.x)),
                        ("y1".into(), n(a.y)),
                        ("x2".into(), n(b.x)),
                        ("y2".into(), n(b.y)),
                    ]
                },
            )
        }
        "polygon" | "polyline" => {
            let sp = subpaths.first()?;
            let want_closed = tag == "polygon";
            if subpaths.len() != 1
                || sp.closed != want_closed
                || any_handles(sp)
                || sp.nodes.len() < 2
            {
                return None; // a bezier handle or open/closed flip → no longer a straight poly
            }
            let points = sp
                .nodes
                .iter()
                .map(|nd| format!("{},{}", n(nd.point.x), n(nd.point.y)))
                .collect::<Vec<_>>()
                .join(" ");
            Some(vec![("points".into(), points)])
        }
        _ => None,
    }
}

/// One node in the document tree. Serde round-trips it for persistence (localStorage) + the
/// undo Snapshot — the tree is the mutable structural model, not a derived view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Node {
    /// An element: its parsed tag name + attributes (for editing), plus the verbatim open/close
    /// tag text so an unedited node re-emits byte-for-byte. `edited` flips emit to regenerate.
    Element {
        /// Stable in-memory identity — the address human clicks *and* LLM/MCP ops both use, so
        /// they hit the same node under concurrent edits (positional paths desync; ids don't).
        /// Assigned at parse in walk order; never re-derived; not emitted to SVG (pure handle).
        uid: String,
        tag: String,
        attrs: Vec<(String, String)>,
        original_open: String,
        original_close: String,
        children: Vec<Node>,
        edited: bool,
        /// Show/hide this node + its subtree (structural op `SetNodeHidden`) → `display="none"`
        /// on export, skipped in the render.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        hidden: bool,
        /// In-app-created node (a drawn path/shape or a group made in the editor) — it has no
        /// verbatim source, so it always regenerates on emit and projects with `added: true`
        /// (drives the STYLE panel's whole-style editing). Parsed nodes are `false`.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        added: bool,
        /// When `Some("union"/"subtract"/"intersect"/"exclude")` this `<g>` is a **live boolean**:
        /// its element children are editable operands and it renders/exports the *computed* boolean
        /// of them (recomputed live) instead of the children themselves. `None` = a plain group.
        #[serde(rename = "booleanOp", default, skip_serializing_if = "Option::is_none")]
        boolean_op: Option<String>,
    },
    /// Verbatim text (incl. whitespace between elements + element text content).
    Text(String),
    /// Verbatim comment, including the `<!-- -->` delimiters.
    Comment(String),
    /// Verbatim anything else nib doesn't structure (processing instruction, CDATA, …).
    Other(String),
}

/// A parsed document: the root `<svg>` element plus the exact text around it (XML declaration,
/// doctype, comments, trailing whitespace) so the whole file round-trips, not just the root.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tree {
    pub prolog: String,
    pub root: Node,
    pub epilog: String,
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}

/// Build a node from a roxmltree node, slicing verbatim spans out of `source`. Children are
/// walked with gap-filling so any inter-child text (whitespace roxmltree may omit as nodes) is
/// captured — guaranteeing the slices cover every byte of the element's span. `next_uid` hands
/// out a fresh stable id per element in walk order.
fn build(node: roxmltree::Node, source: &str, next_uid: &mut usize) -> Node {
    if node.is_comment() {
        let r = node.range();
        return Node::Comment(source[r].to_string());
    }
    if node.is_text() {
        let r = node.range();
        return Node::Text(source[r].to_string());
    }
    if !node.is_element() {
        let r = node.range();
        return Node::Other(source[r].to_string());
    }

    let uid = format!("n{}", *next_uid);
    *next_uid += 1;
    let r = node.range();
    let tag = node.tag_name().name().to_string();
    let attrs: Vec<(String, String)> = node
        .attributes()
        .map(|a| (a.name().to_string(), a.value().to_string()))
        .collect();
    let kids: Vec<roxmltree::Node> = node.children().collect();

    if kids.is_empty() {
        // Self-closing (`<path/>`) or empty (`<g></g>`): the whole span is the "open" text and
        // there is no separate close — emitting `original_open` reproduces it exactly.
        return Node::Element {
            uid,
            tag,
            attrs,
            original_open: source[r.clone()].to_string(),
            original_close: String::new(),
            children: Vec::new(),
            edited: false,
            hidden: false,
            added: false,
            boolean_op: None,
        };
    }

    let first = kids.first().unwrap().range().start;
    let last = kids.last().unwrap().range().end;
    let original_open = source[r.start..first].to_string();
    let original_close = source[last..r.end].to_string();

    let mut children = Vec::new();
    let mut cursor = first;
    for k in kids {
        let kr = k.range();
        if kr.start > cursor {
            children.push(Node::Text(source[cursor..kr.start].to_string())); // gap = whitespace
        }
        children.push(build(k, source, next_uid));
        cursor = kr.end;
    }

    Node::Element {
        uid,
        tag,
        attrs,
        original_open,
        original_close,
        children,
        edited: false,
        hidden: false,
        added: false,
        boolean_op: None,
    }
}

/// Parse an SVG source string into the full document tree. Errors on markup with no `<svg>`
/// root or that fails to parse (mirrors `parse_svg`).
pub fn parse_tree(source: &str) -> Result<Tree, String> {
    let doc =
        roxmltree::Document::parse(source).map_err(|e| format!("could not parse SVG: {e}"))?;
    let root_el = doc.root_element();
    if root_el.tag_name().name() != "svg" {
        return Err("no <svg> root element found".to_string());
    }
    let r = root_el.range();
    let mut next_uid = 0;
    Ok(Tree {
        prolog: source[..r.start].to_string(),
        root: build(root_el, source, &mut next_uid),
        epilog: source[r.end..].to_string(),
    })
}

/// Regenerate an element's open tag from its parsed tag + attributes (used when `edited`).
fn regen_open(tag: &str, attrs: &[(String, String)], self_closing: bool) -> String {
    let a: String = attrs
        .iter()
        .map(|(k, v)| format!(" {k}=\"{}\"", escape_attr(v)))
        .collect();
    if self_closing {
        format!("<{tag}{a}/>")
    } else {
        format!("<{tag}{a}>")
    }
}

/// Insert `display="none"` into an open tag (before its closing `>` / `/>`), unless it already
/// carries a `display`. Keeps the rest of the tag verbatim.
fn with_display_none(open: &str) -> String {
    if open.contains("display=") {
        return open.to_string();
    }
    let cut = if let Some(i) = open.rfind("/>") {
        i
    } else if let Some(i) = open.rfind('>') {
        i
    } else {
        return open.to_string();
    };
    format!(
        "{} display=\"none\"{}",
        open[..cut].trim_end(),
        &open[cut..]
    )
}

/// Read an element node's presentation style (the `STYLE_KEYS` attrs) into a map — the paint an
/// operand contributes to a baked/rendered boolean result.
fn node_style(attrs: &[(String, String)]) -> IndexMap<String, String> {
    let mut style = IndexMap::new();
    for key in STYLE_KEYS {
        if let Some(v) = attr(attrs, key) {
            style.insert(key.to_string(), v.to_string());
        }
    }
    style
}

/// Turn a boolean group's element children into operand `PathElement`s (skipping hidden ones and
/// non-shape/opaque nodes) — the live inputs a boolean is computed from.
fn node_operands(children: &[Node]) -> Vec<PathElement> {
    children
        .iter()
        .filter_map(|c| match c {
            Node::Element {
                tag,
                attrs,
                hidden: false,
                ..
            } => shape_subpaths(tag, attrs).map(|subpaths| PathElement {
                id: String::new(),
                uid: String::new(),
                index: 0,
                original_d: String::new(),
                subpaths,
                edited: false,
                added: true,
                attributes: Some(node_style(attrs)),
                style_override: None,
                original_tag: None,
                deleted: false,
                renamed: false,
                hidden: false,
            }),
            _ => None,
        })
        .collect()
}

/// Compute a live-boolean group's baked `<path .../>` from its operand children. `None` when the
/// boolean can't be formed (< 2 operands or empty geometry) so the caller emits the operands.
fn baked_boolean(children: &[Node], op: &str, precision: usize) -> Option<String> {
    let operands = node_operands(children);
    if operands.len() < 2 {
        return None;
    }
    let refs: Vec<&PathElement> = operands.iter().collect();
    let subpaths = crate::model::booleans::boolean(op, &refs)?;
    if subpaths.is_empty() {
        return None;
    }
    // Subject paint = the first operand that actually fills (so a filled shape + a stroke-only
    // cutter keeps the fill), else the first operand.
    let subject = operands
        .iter()
        .find(|p| {
            p.attributes
                .as_ref()
                .and_then(|a| a.get("fill"))
                .map(|f| f != "none")
                .unwrap_or(true)
        })
        .or_else(|| operands.first());
    let style: String = subject
        .and_then(|p| p.attributes.as_ref())
        .map(|a| a.iter().map(|(k, v)| format!(" {k}=\"{v}\"")).collect())
        .unwrap_or_default();
    Some(format!(
        "<path d=\"{}\"{} />",
        path_to_d_prec(&subpaths, precision),
        style
    ))
}

fn emit_prec(node: &Node, out: &mut String, precision: usize) {
    match node {
        Node::Text(s) | Node::Comment(s) | Node::Other(s) => out.push_str(s),
        Node::Element {
            tag,
            attrs,
            original_open,
            original_close,
            children,
            edited,
            hidden,
            boolean_op,
            uid: _,
            added: _,
        } => {
            let open = if *edited {
                regen_open(tag, attrs, original_close.is_empty())
            } else {
                original_open.clone()
            };
            out.push_str(&if *hidden {
                with_display_none(&open)
            } else {
                open
            });
            // A live-boolean `<g>` bakes to ONE computed `<path>` — SVG can't express the live op,
            // so export renders correctly everywhere; the liveness lives in nib's model. Falls back
            // to emitting the operands if the boolean can't be formed (< 2 operands / empty).
            match boolean_op
                .as_deref()
                .and_then(|op| baked_boolean(children, op, precision))
            {
                Some(baked) => {
                    out.push_str("\n    ");
                    out.push_str(&baked);
                    out.push('\n');
                }
                None => {
                    for c in children {
                        emit_prec(c, out, precision);
                    }
                }
            }
            out.push_str(original_close);
        }
    }
}

/// Re-emit the tree to SVG text. Unedited nodes emit verbatim (byte-for-byte); edited elements
/// regenerate their open tag from `tag` + `attrs`. Baked booleans + drawn geometry use precision 3.
pub fn serialize_tree(tree: &Tree) -> String {
    serialize_tree_prec(tree, 3)
}

/// Re-emit the tree at a chosen numeric precision (for regenerated `d`/baked-boolean geometry).
pub fn serialize_tree_prec(tree: &Tree, precision: usize) -> String {
    let mut out = String::with_capacity(tree.prolog.len() + tree.epilog.len() + 256);
    out.push_str(&tree.prolog);
    emit_prec(&tree.root, &mut out, precision);
    out.push_str(&tree.epilog);
    out
}

fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn collect_paths(node: &Node, out: &mut Vec<PathElement>) {
    let Node::Element {
        uid,
        tag,
        attrs,
        original_open,
        children,
        added,
        ..
    } = node
    else {
        return;
    };
    // A `<defs>`/clipPath/mask/… subtree renders only by reference — never project its inner shapes
    // as editable paths (nor recurse), or they'd become phantom rows in the panel + `doc.paths`.
    if DEF_CONTAINERS.contains(&tag.as_str()) {
        return;
    }
    // `<path>` always projects (even an empty `d`, matching the flat parser); the other
    // primitives project only when they have valid geometry (else stay opaque/uneditable).
    let subpaths = if tag == "path" {
        Some(parse_path_d(attr(attrs, "d").unwrap_or("")))
    } else {
        shape_subpaths(tag, attrs)
    };
    if let Some(subpaths) = subpaths {
        let index = out.len();
        // A path keeps the flat parser's `path-N` fallback name; shapes get a friendly
        // `rect-N`/`circle-N`. Explicit `id` attr wins for either.
        let id = attr(attrs, "id")
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{tag}-{index}"));
        // A drawn (added) node has no source tag: its `attributes` carry the *whole* style (every
        // non-geometry attr), edited directly by the STYLE panel and regenerated on emit. An
        // imported node keeps only the parsed `STYLE_KEYS` subset (its full tag is preserved
        // verbatim); style edits accumulate in `style_override`.
        let style: IndexMap<String, String> = if *added {
            attrs
                .iter()
                .filter(|(k, _)| {
                    k != "id" && k != "d" && !GEOMETRY_ATTRS.contains(&k.as_str())
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        } else {
            node_style(attrs)
        };
        // original_d is the source `d` for a path; shapes have none (their `d` is regenerated
        // on edit via reconcile).
        let original_d = if tag == "path" {
            attr(attrs, "d").unwrap_or("").to_string()
        } else {
            String::new()
        };
        out.push(PathElement {
            id,
            uid: uid.clone(),
            index,
            subpaths,
            attributes: Some(style),
            original_tag: if *added {
                None
            } else {
                Some(original_open.clone())
            },
            original_d,
            edited: false,
            added: *added,
            style_override: None,
            deleted: false,
            renamed: false,
            hidden: false,
        });
    }
    for c in children {
        collect_paths(c, out);
    }
}

fn set_or_push(attrs: &mut Vec<(String, String)>, key: &str, value: &str) {
    match attrs.iter_mut().find(|(k, _)| k == key) {
        Some((_, v)) => *v = value.to_string(),
        None => attrs.push((key.to_string(), value.to_string())),
    }
}

fn reconcile_node(
    node: &mut Node,
    by_uid: &HashMap<&str, &PathElement>,
    precision: usize,
    normalize: bool,
) {
    let Node::Element {
        uid,
        tag,
        attrs,
        original_open,
        original_close,
        children,
        edited,
        hidden,
        added,
        ..
    } = node
    else {
        return;
    };
    // Normalized export: regenerate every element from its parsed tag+attrs (canonical), and force
    // every editable shape to a `<path>` (below) — a clean, fully-derived copy vs the byte-preserving
    // save. Text nodes still emit verbatim, so whitespace/structure is kept.
    if normalize {
        *edited = true;
    }
    if let Some(p) = by_uid.get(uid.as_str()) {
        if p.deleted {
            // Drop the element: blank its tags + children so it emits nothing (the flat model
            // soft-deletes; this reflects that in the tree without touching indices).
            original_open.clear();
            original_close.clear();
            children.clear();
            *edited = false;
            return;
        }
        if *added {
            // A drawn node regenerates wholesale from the model: `attributes` is its whole style
            // (+ id if named) and the current geometry `d` (drawn shapes are always `<path>`).
            let mut regen: Vec<(String, String)> = Vec::new();
            if p.renamed {
                regen.push(("id".to_string(), p.id.clone()));
            }
            if let Some(a) = &p.attributes {
                for (k, v) in a {
                    regen.push((k.clone(), v.clone()));
                }
            }
            regen.push(("d".to_string(), path_to_d_prec(&p.subpaths, precision)));
            *attrs = regen;
            *tag = "path".to_string();
            *edited = true;
        } else {
            // `normalize` forces every editable shape to a canonical `<path>` (skip the refit that
            // would keep a `<rect>`); otherwise only an edited path re-serializes.
            if p.edited || normalize {
                if tag == "path" {
                    set_or_push(attrs, "d", &path_to_d_prec(&p.subpaths, precision));
                } else if !normalize && let Some(geo) = refit(tag, &p.subpaths, precision) {
                    // A move/resize keeps the primitive in form → re-emit it as itself with
                    // updated geometry attrs (a `<rect>` stays a `<rect>`), preserving clean markup.
                    for (k, v) in geo {
                        set_or_push(attrs, &k, &v);
                    }
                } else {
                    // The edit broke the form (e.g. a dragged corner), or we're normalizing →
                    // become a `<path>` (its geometry is the `d`; shape attrs would be dead weight).
                    attrs.retain(|(k, _)| !GEOMETRY_ATTRS.contains(&k.as_str()));
                    *tag = "path".to_string();
                    set_or_push(attrs, "d", &path_to_d_prec(&p.subpaths, precision));
                }
                *edited = true;
            }
            if p.renamed {
                set_or_push(attrs, "id", &p.id);
                *edited = true;
            }
            if let Some(so) = &p.style_override {
                for (k, v) in so {
                    set_or_push(attrs, k, v);
                    *edited = true;
                }
            }
        }
        // Per-path hide (SetPathHidden) exports as `display="none"`, OR'd with any structural hide.
        if p.hidden {
            *hidden = true;
        }
    }
    for c in children {
        reconcile_node(c, by_uid, precision, normalize);
    }
}

impl Tree {
    /// The root `<svg>`'s children as UI render nodes — what the canvas draws declaratively (the
    /// svg element itself is the canvas's own viewport, so only its children are rendered).
    pub fn render_children(&self) -> Vec<RenderNode> {
        match &self.root {
            Node::Element { children, .. } => children.iter().filter_map(to_render).collect(),
            _ => Vec::new(),
        }
    }

    /// Project the flat `<path>` view the editor/frontend runs on out of the tree, in document
    /// order — the bridge that lets the `Editor` be tree-backed while the paths UI keeps working.
    /// Every editable shape element (`path`/`rect`/`circle`/… via `shape_subpaths`) becomes a
    /// `PathElement` carrying its node `uid`; non-editable elements stay opaque tree nodes.
    pub fn project_paths(&self) -> Vec<PathElement> {
        let mut out = Vec::new();
        collect_paths(&self.root, &mut out);
        out
    }

    /// Write the flat paths view's edits back onto the tree, matched to nodes by stable `uid`
    /// (robust to reorder/grouping): an edited path/shape regenerates its `d` (a shape converts to
    /// `<path>`), renamed → `id`, style overrides → attrs, deleted → dropped — each marking only
    /// that node edited so siblings stay verbatim. The return direction of `project_paths`; drawn
    /// (added) paths have no `uid`/node and are appended separately on export.
    pub fn reconcile_paths(&mut self, paths: &[PathElement], precision: usize) {
        self.reconcile_paths_opt(paths, precision, false);
    }

    /// Like [`reconcile_paths`], but `normalize` regenerates every node canonically and forces
    /// every editable shape to a `<path>` — the basis of the "export normalized copy" action.
    pub fn reconcile_paths_opt(&mut self, paths: &[PathElement], precision: usize, normalize: bool) {
        let by_uid: HashMap<&str, &PathElement> = paths
            .iter()
            .filter(|p| !p.uid.is_empty())
            .map(|p| (p.uid.as_str(), p))
            .collect();
        reconcile_node(&mut self.root, &by_uid, precision, normalize);
    }

    /// Show/hide the element with stable id `uid` (structural op). Returns whether it was found.
    pub fn set_hidden(&mut self, uid: &str, hidden: bool) -> bool {
        self.root
            .find_by_uid_mut(uid)
            .map(|n| n.set_hidden(hidden))
            .unwrap_or(false)
    }

    /// Wrap the elements `uids` (which must share one parent) in a new `<g uid id="name">` at the
    /// first member's position. Returns false if they aren't all siblings under one node.
    pub fn group(&mut self, uids: &[String], new_uid: &str, name: &str) -> bool {
        group_in(&mut self.root, uids, new_uid, name)
    }

    /// Dissolve the group `uid`, splicing its children into its parent in place. Returns false if
    /// the uid isn't an element with children.
    pub fn ungroup(&mut self, uid: &str) -> bool {
        ungroup_in(&mut self.root, uid)
    }

    /// Swap the node `uid` with its adjacent element sibling — `forward` (toward the end / higher
    /// z) or backward. No-op at the end of the run. Returns whether the node was found.
    pub fn reorder(&mut self, uid: &str, forward: bool) -> bool {
        reorder_in(&mut self.root, uid, forward)
    }

    /// Set (`Some`) or remove (`None`) an attribute on the element `uid`, marking it edited so it
    /// regenerates on emit. Returns whether the node was found + changed.
    pub fn set_node_attr(&mut self, uid: &str, key: &str, value: Option<&str>) -> bool {
        match self.root.find_by_uid_mut(uid) {
            Some(Node::Element { attrs, edited, .. }) => {
                match value {
                    Some(v) => match attrs.iter_mut().find(|(k, _)| k == key) {
                        Some((_, val)) => *val = v.to_string(),
                        None => attrs.push((key.to_string(), v.to_string())),
                    },
                    None => {
                        let before = attrs.len();
                        attrs.retain(|(k, _)| k != key);
                        if attrs.len() == before {
                            return false; // nothing to remove
                        }
                    }
                }
                *edited = true;
                true
            }
            _ => false,
        }
    }

    /// Replace a text-bearing element's content with `text` (one child text node) — e.g. editing a
    /// `<text>` label. Marks it edited. Returns whether the node was found.
    pub fn set_node_text(&mut self, uid: &str, text: &str) -> bool {
        match self.root.find_by_uid_mut(uid) {
            Some(Node::Element {
                tag,
                children,
                edited,
                original_close,
                ..
            }) => {
                // A self-closing element (no close tag) needs a real open/close pair to hold text,
                // else the regenerated open would be `<tag/>` and the child would fall outside it.
                if original_close.is_empty() {
                    *original_close = format!("</{tag}>");
                }
                *children = vec![Node::Text(text.to_string())];
                *edited = true;
                true
            }
            _ => false,
        }
    }

    /// Remove `<linearGradient>`/`<radialGradient>` nodes whose `id` is in `ids` — the model has
    /// adopted them into `doc.gradients` (editable), so they re-emit from there via `inject_defs`;
    /// dropping the source node avoids a duplicate id. A no-op when `ids` is empty, so an unedited
    /// document (nothing adopted) still round-trips byte-for-byte.
    pub fn remove_gradient_defs(&mut self, ids: &std::collections::HashSet<String>) {
        fn is_adopted(n: &Node, ids: &std::collections::HashSet<String>) -> bool {
            matches!(n, Node::Element { tag, attrs, .. }
                if (tag == "linearGradient" || tag == "radialGradient")
                    && attrs.iter().any(|(k, v)| k == "id" && ids.contains(v)))
        }
        fn walk(node: &mut Node, ids: &std::collections::HashSet<String>) {
            if let Node::Element { children, .. } = node {
                children.retain(|c| !is_adopted(c, ids));
                for c in children.iter_mut() {
                    walk(c, ids);
                }
            }
        }
        if !ids.is_empty() {
            walk(&mut self.root, ids);
        }
    }

    /// Set (`Some`) or clear (`None`) the live-boolean op on the group node `uid`. Returns whether
    /// it was found (any element node can carry the marker; the frontend only sets it on groups).
    pub fn set_boolean(&mut self, uid: &str, op: Option<String>) -> bool {
        match self.root.find_by_uid_mut(uid) {
            Some(Node::Element { boolean_op, .. }) => {
                *boolean_op = op;
                true
            }
            _ => false,
        }
    }

    /// Append a freshly-built drawn node as the last child of the root `<svg>` (top of z-order),
    /// on its own indented line. A childless root (self-closing `<svg/>` or empty `<svg></svg>`,
    /// where the close tag folded into `original_open`) is regenerated into a real open + close so
    /// the appended node lands *inside* the root.
    pub fn append_drawn(&mut self, node: Node) {
        if let Node::Element {
            tag,
            attrs,
            children,
            original_open,
            original_close,
            edited,
            ..
        } = &mut self.root
        {
            if original_close.is_empty() {
                *original_open = regen_open(tag, attrs, false);
                *original_close = format!("</{tag}>");
                *edited = true;
            }
            children.push(Node::Text("\n  ".to_string()));
            children.push(node);
        }
    }

    /// A stable uid not already used by any element in the tree — `u0`, `u1`, … Drawn nodes and
    /// groups mint one so their identity is unique for reconcile/addressing.
    pub fn fresh_uid(&self) -> String {
        let mut used = std::collections::HashSet::new();
        fn walk<'a>(n: &'a Node, used: &mut std::collections::HashSet<&'a str>) {
            if let Node::Element { uid, children, .. } = n {
                used.insert(uid.as_str());
                for c in children {
                    walk(c, used);
                }
            }
        }
        walk(&self.root, &mut used);
        let mut k = 0usize;
        loop {
            let candidate = format!("u{k}");
            if !used.contains(candidate.as_str()) {
                return candidate;
            }
            k += 1;
        }
    }
}

/// Build a drawn `<path>` node from its style `attributes` + geometry `d`. `edited: true` +
/// `added: true` so it always regenerates on emit and projects with `added`; no verbatim source.
pub fn make_path_node(uid: &str, attributes: &IndexMap<String, String>, d: &str) -> Node {
    let mut attrs: Vec<(String, String)> = attributes
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    attrs.push(("d".to_string(), d.to_string()));
    Node::Element {
        uid: uid.to_string(),
        tag: "path".to_string(),
        attrs,
        original_open: String::new(),
        original_close: String::new(),
        children: Vec::new(),
        edited: true,
        hidden: false,
        added: true,
        boolean_op: None,
    }
}

fn reorder_in(node: &mut Node, uid: &str, forward: bool) -> bool {
    if let Node::Element { children, .. } = node {
        if let Some(i) = children.iter().position(|c| c.uid() == Some(uid)) {
            // Adjacent *element* sibling (skipping whitespace text nodes between them).
            let j = if forward {
                (i + 1..children.len()).find(|&k| matches!(children[k], Node::Element { .. }))
            } else {
                (0..i)
                    .rev()
                    .find(|&k| matches!(children[k], Node::Element { .. }))
            };
            if let Some(j) = j {
                children.swap(i, j);
            }
            return true;
        }
        for c in children.iter_mut() {
            if reorder_in(c, uid, forward) {
                return true;
            }
        }
    }
    false
}

fn group_in(node: &mut Node, uids: &[String], new_uid: &str, name: &str) -> bool {
    if let Node::Element { children, .. } = node {
        let positions: Vec<usize> = children
            .iter()
            .enumerate()
            .filter(|(_, c)| c.uid().is_some_and(|u| uids.iter().any(|x| x == u)))
            .map(|(i, _)| i)
            .collect();
        // All uids are direct children here → group them at the first's slot.
        if !uids.is_empty() && positions.len() == uids.len() {
            let at = positions[0];
            let mut grabbed: Vec<Node> = positions
                .iter()
                .rev()
                .map(|&i| children.remove(i))
                .collect();
            grabbed.reverse();
            let g = Node::Element {
                uid: new_uid.to_string(),
                tag: "g".to_string(),
                attrs: vec![("id".to_string(), name.to_string())],
                original_open: format!("<g id=\"{}\">", escape_attr(name)),
                original_close: "</g>".to_string(),
                children: grabbed,
                edited: false,
                hidden: false,
                added: true,
                boolean_op: None,
            };
            children.insert(at.min(children.len()), g);
            return true;
        }
        for c in children.iter_mut() {
            if group_in(c, uids, new_uid, name) {
                return true;
            }
        }
    }
    false
}

fn ungroup_in(node: &mut Node, uid: &str) -> bool {
    if let Node::Element { children, .. } = node {
        if let Some(i) = children.iter().position(|c| c.uid() == Some(uid)) {
            let inner = match &children[i] {
                Node::Element { children: gc, .. } => gc.clone(),
                _ => return false,
            };
            children.splice(i..=i, inner);
            return true;
        }
        for c in children.iter_mut() {
            if ungroup_in(c, uid) {
                return true;
            }
        }
    }
    false
}

/// A UI-facing render node — the tree the canvas draws declaratively. Elements carry their tag +
/// attrs (as an object) + children + `uid`; text carries its content. Comments / PIs are dropped
/// (they don't paint). Editable shape elements (matched by `uid` to the live `doc.paths`) are
/// re-drawn from the model as `<path>`; everything else renders verbatim from these attrs.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RenderNode {
    Element {
        uid: String,
        tag: String,
        attrs: IndexMap<String, String>,
        children: Vec<RenderNode>,
        hidden: bool,
        /// Set on a live-boolean `<g>` — the canvas paints the computed result (from
        /// `booleanResults`, keyed by this node's `uid`) instead of recursing into the operands.
        #[serde(rename = "booleanOp", skip_serializing_if = "Option::is_none")]
        boolean_op: Option<String>,
    },
    Text {
        text: String,
    },
}

fn to_render(node: &Node) -> Option<RenderNode> {
    match node {
        Node::Element {
            uid,
            tag,
            attrs,
            children,
            hidden,
            boolean_op,
            ..
        } => Some(RenderNode::Element {
            uid: uid.clone(),
            tag: tag.clone(),
            attrs: attrs.iter().cloned().collect(),
            children: children.iter().filter_map(to_render).collect(),
            hidden: *hidden,
            boolean_op: boolean_op.clone(),
        }),
        Node::Text(s) => Some(RenderNode::Text { text: s.clone() }),
        Node::Comment(_) | Node::Other(_) => None,
    }
}

impl Node {
    /// Set (or add) an attribute and mark the element edited so it regenerates on emit. Returns
    /// false for non-element nodes.
    pub fn set_attr(&mut self, key: &str, value: &str) -> bool {
        if let Node::Element { attrs, edited, .. } = self {
            match attrs.iter_mut().find(|(k, _)| k == key) {
                Some((_, v)) => *v = value.to_string(),
                None => attrs.push((key.to_string(), value.to_string())),
            }
            *edited = true;
            true
        } else {
            false
        }
    }

    /// This element's stable uid (`None` for non-element nodes).
    pub fn uid(&self) -> Option<&str> {
        match self {
            Node::Element { uid, .. } => Some(uid),
            _ => None,
        }
    }

    /// Show/hide this element (structural op). Returns false for non-element nodes.
    pub fn set_hidden(&mut self, value: bool) -> bool {
        if let Node::Element { hidden, .. } = self {
            *hidden = value;
            true
        } else {
            false
        }
    }

    /// Depth-first search for the element with stable id `uid` — the addressing primitive tree
    /// ops (and the MCP surface) resolve a target through.
    pub fn find_by_uid_mut(&mut self, uid: &str) -> Option<&mut Node> {
        let is_match = matches!(self, Node::Element { uid: u, .. } if u == uid);
        if is_match {
            return Some(self);
        }
        if let Node::Element { children, .. } = self {
            for c in children {
                if let Some(found) = c.find_by_uid_mut(uid) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Depth-first search for the first element with `id`.
    pub fn find_by_id_mut(&mut self, id: &str) -> Option<&mut Node> {
        // Check this node's match first (borrow ends), then recurse — two phases to satisfy the
        // borrow checker (can't hold the `children` borrow while returning `self`).
        let is_match = matches!(self, Node::Element { attrs, .. } if attrs.iter().any(|(k, v)| k == "id" && v == id));
        if is_match {
            return Some(self);
        }
        if let Node::Element { children, .. } = self {
            for c in children {
                if let Some(found) = c.find_by_id_mut(id) {
                    return Some(found);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORPUS: &[&str] = &[
        include_str!("../../tests/fixtures/minimal.svg"),
        include_str!("../../tests/fixtures/icon-group.svg"),
        include_str!("../../tests/fixtures/gradient.svg"),
        include_str!("../../tests/fixtures/mixed-elements.svg"),
        include_str!("../../tests/fixtures/style-block.svg"),
        include_str!("../../tests/fixtures/transforms.svg"),
        include_str!("../../tests/fixtures/prolog.svg"),
        include_str!("../../tests/fixtures/shapes.svg"),
        include_str!("../../tests/fixtures/defs.svg"),
    ];

    #[test]
    fn parse_serialize_is_byte_for_byte_on_the_corpus() {
        for (i, src) in CORPUS.iter().enumerate() {
            let tree = parse_tree(src).unwrap_or_else(|e| panic!("fixture {i}: {e}"));
            assert_eq!(&serialize_tree(&tree), src, "fixture {i} not byte-for-byte");
        }
    }

    #[test]
    fn captures_non_path_elements_as_structured_nodes() {
        // The flat model drops rect/circle/text into an opaque string; the tree structures them.
        let tree = parse_tree(CORPUS[3]).unwrap(); // mixed-elements
        let Node::Element { children, tag, .. } = &tree.root else {
            panic!("root not an element");
        };
        assert_eq!(tag, "svg");
        let tags: Vec<&str> = children
            .iter()
            .filter_map(|c| match c {
                Node::Element { tag, .. } => Some(tag.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(tags, ["rect", "circle", "path", "text"]);
    }

    #[test]
    fn every_element_gets_a_unique_stable_uid_not_leaked_to_output() {
        let src = CORPUS[1]; // icon-group: nested <g> + 3 paths
        let tree = parse_tree(src).unwrap();
        // collect all element uids
        fn walk<'a>(n: &'a Node, out: &mut Vec<&'a str>) {
            if let Node::Element { uid, children, .. } = n {
                out.push(uid);
                for c in children {
                    walk(c, out);
                }
            }
        }
        let mut uids = Vec::new();
        walk(&tree.root, &mut uids);
        assert!(uids.len() >= 5, "svg + g + 3 paths: {uids:?}");
        let unique: std::collections::HashSet<_> = uids.iter().collect();
        assert_eq!(unique.len(), uids.len(), "uids must be unique: {uids:?}");
        // uids are a pure in-memory handle — never written to the SVG.
        assert!(
            !serialize_tree(&tree).contains("n0"),
            "uid leaked into output"
        );
    }

    #[test]
    fn set_hidden_hides_a_node_on_export_and_in_render() {
        let mut tree = parse_tree(CORPUS[1]).unwrap(); // icon-group: <g id="toolbar"> with 3 paths
        let uid = tree
            .root
            .find_by_id_mut("toolbar")
            .and_then(|n| n.uid().map(str::to_string))
            .unwrap();
        assert!(tree.set_hidden(&uid, true));

        let out = serialize_tree(&tree);
        assert!(
            out.contains("<g id=\"toolbar\""),
            "group still there: {out}"
        );
        assert!(
            out.contains("display=\"none\""),
            "group hidden on export: {out}"
        );
        assert!(
            out.contains(r#"<path d="M 3 6 L 21 6"/>"#),
            "children verbatim: {out}"
        );

        // The render node carries the hidden flag so the canvas can skip the subtree.
        fn find<'a>(nodes: &'a [RenderNode], uid: &str) -> Option<bool> {
            for n in nodes {
                if let RenderNode::Element {
                    uid: u,
                    hidden,
                    children,
                    ..
                } = n
                {
                    if u == uid {
                        return Some(*hidden);
                    }
                    if let Some(h) = find(children, uid) {
                        return Some(h);
                    }
                }
            }
            None
        }
        assert_eq!(find(&tree.render_children(), &uid), Some(true));

        // Unhiding restores byte-for-byte.
        assert!(tree.set_hidden(&uid, false));
        assert_eq!(serialize_tree(&tree), CORPUS[1]);
    }

    #[test]
    fn set_node_attr_and_text_edit_a_non_shape_element() {
        let mut tree = parse_tree(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="10" y="20" font-size="12">hi</text></svg>"#,
        )
        .unwrap();
        fn uid_of<'a>(n: &'a Node, tag: &str) -> Option<&'a str> {
            if let Node::Element {
                tag: t,
                uid,
                children,
                ..
            } = n
            {
                if t == tag {
                    return Some(uid);
                }
                for c in children {
                    if let Some(u) = uid_of(c, tag) {
                        return Some(u);
                    }
                }
            }
            None
        }
        let uid = uid_of(&tree.root, "text").unwrap().to_string();
        // Edit a geometry attr + the text content.
        assert!(tree.set_node_attr(&uid, "x", Some("30")));
        assert!(tree.set_node_text(&uid, "world"));
        let out = serialize_tree(&tree);
        assert!(out.contains("x=\"30\""), "{out}");
        assert!(out.contains(">world</text>"), "{out}");
        assert!(!out.contains("hi"), "{out}");
        // Removing an attr drops it; a missing attr is a no-op.
        assert!(tree.set_node_attr(&uid, "font-size", None));
        assert!(!serialize_tree(&tree).contains("font-size"));
        assert!(!tree.set_node_attr(&uid, "font-size", None));
        assert!(!tree.set_node_attr("nope", "x", Some("1")));
    }

    #[test]
    fn find_by_uid_resolves_and_edits_the_addressed_node() {
        let mut tree = parse_tree(CORPUS[1]).unwrap();
        let uid = tree.root.uid().unwrap().to_string(); // the <svg> root
        let node = tree
            .root
            .find_by_uid_mut(&uid)
            .expect("resolve root by uid");
        assert!(node.set_attr("data-x", "1"));
        assert!(serialize_tree(&tree).contains("data-x=\"1\""));
        assert!(tree.root.find_by_uid_mut("nonexistent").is_none());
    }

    #[test]
    fn project_paths_matches_the_flat_parser_for_path_only_docs() {
        // For path-only fixtures the projection reproduces the flat parser exactly (regression
        // guard). Only the new `uid` differs, so compare with uid cleared. Fixtures with
        // primitives (mixed-elements #3, shapes #7, defs #8 — a rect) project extra editable paths.
        for (i, src) in CORPUS.iter().enumerate() {
            if i == 3 || i == 7 || i == 8 {
                continue;
            }
            let projected: Vec<_> = parse_tree(src)
                .unwrap()
                .project_paths()
                .into_iter()
                .map(|mut p| {
                    p.uid = String::new();
                    p
                })
                .collect();
            let flat = crate::model::document::parse_svg(src).unwrap().paths;
            assert_eq!(
                projected, flat,
                "fixture {i}: projected paths != flat parser"
            );
        }
    }

    #[test]
    fn shapes_sample_projects_every_primitive_as_editable() {
        // The samples/shapes.svg test file: one of each primitive + a path — all seven project
        // as editable paths (confirms the file users open is fully editable).
        let paths = parse_tree(CORPUS[7]).unwrap().project_paths();
        let ids: Vec<&str> = paths.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "rect-0",
                "circle-1",
                "ellipse-2",
                "line-3",
                "polygon-4",
                "polyline-5",
                "path-6"
            ]
        );
        assert!(
            paths
                .iter()
                .all(|p| !p.subpaths.is_empty() && !p.uid.is_empty())
        );
    }

    #[test]
    fn project_paths_includes_editable_primitives() {
        // mixed-elements: <rect>, <circle>, <path>, <text>. The three shapes project as editable
        // paths (text stays opaque, not editable).
        let paths = parse_tree(CORPUS[3]).unwrap().project_paths();
        let ids: Vec<&str> = paths.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["rect-0", "circle-1", "path-2"]); // <text> excluded
        assert!(
            paths.iter().all(|p| !p.uid.is_empty()),
            "each projected path carries a uid"
        );
        assert!(paths[0].subpaths[0].closed && paths[0].subpaths[0].nodes.len() == 4); // rect
    }

    #[test]
    fn moving_or_resizing_a_primitive_keeps_it_as_that_primitive() {
        use crate::model::types::Point;
        let mut tree = parse_tree(CORPUS[7]).unwrap(); // shapes fixture
        let mut paths = tree.project_paths();
        let shift = |p: &mut PathElement, dx: f64, dy: f64| {
            for n in &mut p.subpaths[0].nodes {
                n.point = Point::new(n.point.x + dx, n.point.y + dy);
                n.handle_in = n.handle_in.map(|h| Point::new(h.x + dx, h.y + dy));
                n.handle_out = n.handle_out.map(|h| Point::new(h.x + dx, h.y + dy));
            }
            p.edited = true;
        };
        shift(&mut paths[0], 5.0, 5.0); // rect  (20,20) → (25,25)
        shift(&mut paths[1], 10.0, 0.0); // circle cx 160 → 170
        shift(&mut paths[3], 3.0, 0.0); // line  x1 20 → 23
        shift(&mut paths[4], 0.0, 4.0); // polygon
        tree.reconcile_paths(&paths, 2);
        let out = serialize_tree(&tree);
        assert!(
            out.contains("<rect x=\"25\" y=\"25\""),
            "moved rect stays <rect>: {out}"
        );
        assert!(
            out.contains("<circle cx=\"170\""),
            "moved circle stays <circle>: {out}"
        );
        assert!(
            out.contains("<line x1=\"23\""),
            "moved line stays <line>: {out}"
        );
        assert!(
            out.contains("<polygon points=\""),
            "moved polygon stays <polygon>: {out}"
        );
    }

    #[test]
    fn reshaping_a_primitive_off_form_falls_back_to_a_path() {
        use crate::model::types::Point;
        let mut tree = parse_tree(CORPUS[7]).unwrap();
        let mut paths = tree.project_paths();
        // drag one rect corner inward → no longer axis-aligned → must become a <path>
        paths[0].subpaths[0].nodes[0].point = Point::new(45.0, 45.0);
        paths[0].edited = true;
        tree.reconcile_paths(&paths, 2);
        let out = serialize_tree(&tree);
        assert!(!out.contains("<rect"), "reshaped rect → path: {out}");
        assert!(out.contains("<path"), "a <path> is emitted: {out}");
    }

    #[test]
    fn editing_a_primitive_serializes_it_as_a_path_siblings_verbatim() {
        use crate::model::types::Point;
        let mut tree = parse_tree(CORPUS[3]).unwrap(); // rect, circle, path, text
        let mut paths = tree.project_paths();
        // edit the rect (index 0) — move a corner
        paths[0].subpaths[0].nodes[0].point = Point::new(1.0, 1.0);
        paths[0].edited = true;
        tree.reconcile_paths(&paths, 2);
        let out = serialize_tree(&tree);
        assert!(
            !out.contains("<rect"),
            "edited rect converted away from <rect>: {out}"
        );
        assert!(
            out.contains("fill=\"#f90\""),
            "rect's fill carried onto the path: {out}"
        );
        // untouched siblings stay verbatim
        assert!(
            out.contains("<circle cx=\"50\" cy=\"50\" r=\"30\""),
            "circle verbatim: {out}"
        );
        assert!(
            out.contains("<text x=\"20\" y=\"90\""),
            "text verbatim: {out}"
        );
    }

    #[test]
    fn reconcile_writes_flat_edits_back_and_preserves_siblings() {
        use crate::model::types::Point;
        let mut tree = parse_tree(CORPUS[0]).unwrap(); // minimal: one path, fill="#333"
        let mut paths = tree.project_paths();
        // move the first node — the surgical geometry edit the tools make.
        paths[0].subpaths[0].nodes[0].point = Point::new(5.0, 5.0);
        paths[0].edited = true;
        tree.reconcile_paths(&paths, 2);

        let out = serialize_tree(&tree);
        assert!(out.contains("fill=\"#333\""), "style preserved: {out}");
        assert!(
            !out.contains("M 10 10 L 90 10"),
            "d regenerated (not original): {out}"
        );
        assert!(parse_tree(&out).is_ok(), "reconciled output still parses");
        // an unedited reconcile is a no-op → byte-for-byte
        let mut clean = parse_tree(CORPUS[0]).unwrap();
        clean.reconcile_paths(&clean.project_paths(), 2);
        assert_eq!(
            serialize_tree(&clean),
            CORPUS[0],
            "unedited reconcile stays verbatim"
        );
    }

    #[test]
    fn shape_subpaths_converts_every_primitive_to_editable_geometry() {
        let a = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        // rect → 4 closed corners
        let rect = shape_subpaths(
            "rect",
            &a(&[("x", "10"), ("y", "20"), ("width", "30"), ("height", "40")]),
        )
        .unwrap();
        assert_eq!(rect.len(), 1);
        assert!(rect[0].closed);
        assert_eq!(rect[0].nodes.len(), 4);
        // circle / ellipse → closed 4-node bezier
        assert!(
            shape_subpaths("circle", &a(&[("cx", "5"), ("cy", "5"), ("r", "5")])).unwrap()[0]
                .closed
        );
        assert!(
            shape_subpaths(
                "ellipse",
                &a(&[("cx", "5"), ("cy", "5"), ("rx", "5"), ("ry", "3")])
            )
            .unwrap()[0]
                .closed
        );
        // line → open 2 nodes; polygon closed, polyline open
        let line = shape_subpaths(
            "line",
            &a(&[("x1", "0"), ("y1", "0"), ("x2", "9"), ("y2", "9")]),
        )
        .unwrap();
        assert!(!line[0].closed && line[0].nodes.len() == 2);
        assert!(shape_subpaths("polygon", &a(&[("points", "0,0 10,0 5,8")])).unwrap()[0].closed);
        assert!(!shape_subpaths("polyline", &a(&[("points", "0,0 10,0 5,8")])).unwrap()[0].closed);
        // degenerate / unknown → None (stays opaque, never editable)
        assert!(shape_subpaths("rect", &a(&[("width", "0"), ("height", "10")])).is_none());
        assert!(shape_subpaths("text", &a(&[("x", "0")])).is_none());
        // px units tolerated
        assert!(shape_subpaths("rect", &a(&[("width", "30px"), ("height", "40px")])).is_some());
    }

    #[test]
    fn editing_an_attribute_regenerates_only_that_tag() {
        let mut tree = parse_tree(CORPUS[1]).unwrap(); // icon-group, has id="toolbar"
        let g = tree.root.find_by_id_mut("toolbar").expect("group");
        assert!(g.set_attr("stroke", "#f00"));
        let out = serialize_tree(&tree);
        assert!(
            out.contains("stroke=\"#f00\""),
            "edited attr present: {out}"
        );
        // untouched siblings stay verbatim
        assert!(
            out.contains(r#"<path d="M 3 6 L 21 6"/>"#),
            "siblings verbatim: {out}"
        );
    }
}
