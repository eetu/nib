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
//! **wiring it into the `Editor`/frontend — projecting `paths`, migrating ops — is a later E1
//! step.** Byte-for-byte holds *by construction*: the source is partitioned into slices along
//! child boundaries, each owned by exactly one node, so concatenating the slices reproduces it.

/// One node in the document tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// An element: its parsed tag name + attributes (for editing), plus the verbatim open/close
    /// tag text so an unedited node re-emits byte-for-byte. `edited` flips emit to regenerate.
    Element {
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
/// captured — guaranteeing the slices cover every byte of the element's span.
fn build(node: roxmltree::Node, source: &str) -> Node {
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
        children.push(build(k, source));
        cursor = kr.end;
    }

    Node::Element {
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
    Ok(Tree {
        prolog: source[..r.start].to_string(),
        root: build(root_el, source),
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
