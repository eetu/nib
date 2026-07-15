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
//! Built + proven in isolation here (parse→serialize is byte-for-byte on the round-trip corpus);
//! **wiring it into the `Editor`/frontend — projecting `paths`, migrating ops — is in progress
//! (see `project_paths`).** Byte-for-byte holds *by construction*: the source is partitioned into
//! slices along child boundaries, each owned by exactly one node, so concatenating reproduces it.

use indexmap::IndexMap;

use super::document::STYLE_KEYS;
use super::path::{parse_path_d, path_to_d_prec};
use super::types::PathElement;

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
        tag,
        attrs,
        original_open,
        children,
        ..
    } = node
    else {
        return;
    };
    if tag == "path" {
        let index = out.len();
        let d = attr(attrs, "d").unwrap_or("").to_string();
        let id = attr(attrs, "id")
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("path-{index}"));
        let mut style = IndexMap::new();
        for key in STYLE_KEYS {
            if let Some(v) = attr(attrs, key) {
                style.insert(key.to_string(), v.to_string());
            }
        }
        out.push(PathElement {
            id,
            index,
            subpaths: parse_path_d(&d),
            attributes: Some(style),
            original_tag: Some(original_open.clone()),
            original_d: d,
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

fn reconcile(node: &mut Node, paths: &[PathElement], i: &mut usize, precision: usize) {
    // attrs/edited/children are disjoint fields, so we can update the tag *and* recurse here.
    if let Node::Element {
        tag,
        attrs,
        edited,
        children,
        ..
    } = node
    {
        if tag == "path" {
            if let Some(p) = paths.get(*i) {
                if p.edited {
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
            *i += 1;
        }
        for c in children {
            reconcile(c, paths, i, precision);
        }
    }
}

impl Tree {
    /// Project the flat `<path>` view the current editor/frontend runs on out of the tree, in
    /// document order — the bridge that lets the `Editor` become tree-backed while the paths-based
    /// UI keeps working unchanged. Mirrors `parse_svg`'s path extraction, sourced from tree nodes.
    /// (Non-path elements + `<g>` structure carry richer identity on the tree; they flow into the
    /// paths view as E2/E3 land.)
    pub fn project_paths(&self) -> Vec<PathElement> {
        let mut out = Vec::new();
        collect_paths(&self.root, &mut out);
        out
    }

    /// Write the flat paths view's edits back onto the tree's `<path>` nodes (document order), so
    /// `serialize_tree` reflects them: an edited path regenerates its `d`, a renamed one its `id`,
    /// and style overrides merge into attrs — each marking only that node edited (siblings stay
    /// verbatim). The return direction of `project_paths`; together they bridge flat editing ↔ the
    /// tree until ops mutate the tree directly. (Added/deleted paths + layers land with #29's flip.)
    pub fn reconcile_paths(&mut self, paths: &[PathElement], precision: usize) {
        let mut i = 0;
        reconcile(&mut self.root, paths, &mut i, precision);
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
    fn project_paths_matches_the_flat_parser_across_the_corpus() {
        // The tree can reproduce the exact flat `paths` view the current editor/frontend runs on
        // — the bridge for flipping the Editor to tree-backed without changing the paths UI.
        for (i, src) in CORPUS.iter().enumerate() {
            let projected = parse_tree(src).unwrap().project_paths();
            let flat = crate::model::document::parse_svg(src).unwrap().paths;
            assert_eq!(
                projected, flat,
                "fixture {i}: projected paths != flat parser"
            );
        }
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
