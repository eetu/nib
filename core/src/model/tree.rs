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

/// One node in the document tree.
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
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

fn emit(node: &Node, out: &mut String) {
    match node {
        Node::Text(s) | Node::Comment(s) | Node::Other(s) => out.push_str(s),
        Node::Element {
            tag,
            attrs,
            original_open,
            original_close,
            children,
            edited,
            uid: _,
        } => {
            if *edited {
                out.push_str(&regen_open(tag, attrs, original_close.is_empty()));
            } else {
                out.push_str(original_open);
            }
            for c in children {
                emit(c, out);
            }
            out.push_str(original_close);
        }
    }
}

/// Re-emit the tree to SVG text. Unedited nodes emit verbatim (byte-for-byte); edited elements
/// regenerate their open tag from `tag` + `attrs`.
pub fn serialize_tree(tree: &Tree) -> String {
    let mut out = String::with_capacity(tree.prolog.len() + tree.epilog.len() + 256);
    out.push_str(&tree.prolog);
    emit(&tree.root, &mut out);
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
        ..
    } = node
    else {
        return;
    };
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
        let mut style = IndexMap::new();
        for key in STYLE_KEYS {
            if let Some(v) = attr(attrs, key) {
                style.insert(key.to_string(), v.to_string());
            }
        }
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
            original_tag: Some(original_open.clone()),
            original_d,
            edited: false,
            added: false,
            style_override: None,
            deleted: false,
            renamed: false,
            layer: None,
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

fn reconcile_node(node: &mut Node, by_uid: &HashMap<&str, &PathElement>, precision: usize) {
    let Node::Element {
        uid,
        tag,
        attrs,
        original_open,
        original_close,
        children,
        edited,
    } = node
    else {
        return;
    };
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
        if p.edited {
            // An edited primitive becomes a `<path>` (its geometry is now the `d`; the shape
            // attrs would be dead weight), then the regenerated `d` is written.
            if tag != "path" {
                attrs.retain(|(k, _)| !GEOMETRY_ATTRS.contains(&k.as_str()));
                *tag = "path".to_string();
            }
            set_or_push(attrs, "d", &path_to_d_prec(&p.subpaths, precision));
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
    for c in children {
        reconcile_node(c, by_uid, precision);
    }
}

impl Tree {
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
        let by_uid: HashMap<&str, &PathElement> = paths
            .iter()
            .filter(|p| !p.uid.is_empty())
            .map(|p| (p.uid.as_str(), p))
            .collect();
        reconcile_node(&mut self.root, &by_uid, precision);
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
        // primitives (mixed-elements #3, shapes #7) project extra editable paths → checked below.
        for (i, src) in CORPUS.iter().enumerate() {
            if i == 3 || i == 7 {
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
