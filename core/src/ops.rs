//! The operation vocabulary + reducer — the spine of the editor.
//!
//! Every document mutation the tools perform is expressed as a serializable [`Op`], applied
//! by the pure [`apply`] reducer. This one vocabulary backs undo (each committed op is a
//! step), the command palette, the MCP tool surface, and WebSocket sync — the same ops the
//! human UI emits are what an LLM emits and what crosses the wire.
//!
//! Ops are **index-addressed document mutations**, deliberately decoupled from selection,
//! history, clipboard and persistence (those are client/`Editor` concerns): an LLM editing
//! headlessly does not care about the human's selection. Ops carry absolute target values
//! (set-semantics) so applying one is deterministic and order-tolerant.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::model::geometry::{distance, normalize};
use crate::model::path::{close_subpath, insert_node_at, reversed_subpath};
use crate::model::shapes::ellipse_nodes;
use crate::model::types::{NodeRef, NodeType, PathElement, PathNode, Point, Subpath, SvgDocument};

/// Which control handle of a node an op targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Handle {
    In,
    Out,
}

/// A single document mutation. `apply` is total: an op whose target is missing is a no-op.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Op {
    /// Move an anchor, carrying its handles along by the same delta.
    MoveNode { node: NodeRef, to: Point },
    /// Move one control handle. A smooth node keeps the opposite handle collinear (mirrored
    /// direction, its own length preserved).
    MoveHandle {
        node: NodeRef,
        which: Handle,
        to: Point,
    },
    /// Set a node's corner/smooth type.
    SetNodeType { node: NodeRef, node_type: NodeType },
    /// Pen drag: shape the anchor into a smooth node with mirrored handles about `out`.
    SetPenHandles { node: NodeRef, out: Point },

    /// Replace a whole path's geometry (used by transforms).
    SetSubpaths { path: usize, subpaths: Vec<Subpath> },
    /// Translate an entire path (all subpaths' nodes + handles) by a delta.
    MovePathBy { path: usize, dx: f64, dy: f64 },

    /// Insert a node on the segment leaving `segment` at parameter `t` (shape-preserving).
    InsertNode {
        path: usize,
        subpath: usize,
        segment: usize,
        t: f64,
    },
    /// Delete a node; a subpath left with < 2 nodes is dropped, and a path left with no
    /// subpaths is soft-deleted.
    DeleteNode { node: NodeRef },
    /// Append an anchor to the tail of a subpath (pen drawing).
    AppendNode {
        path: usize,
        subpath: usize,
        point: Point,
    },
    /// Reverse a subpath's direction (its former start becomes the tail).
    ReverseSubpath { path: usize, subpath: usize },
    /// Close a subpath's loop by merging its endpoint onto its start (close-by-snap).
    CloseLoop { path: usize, subpath: usize },
    /// Close a subpath (connect last→first) without moving any node (pen "click the start").
    ClosePath { path: usize, subpath: usize },
    /// Resize the ellipse in a subpath to new centre + radii.
    ResizeEllipse {
        path: usize,
        subpath: usize,
        center: Point,
        rx: f64,
        ry: f64,
    },

    /// Append a new drawn path (pen start, circle start, paste). The caller supplies the id
    /// (so all clients agree) and the resolved style.
    AddPath {
        id: String,
        subpaths: Vec<Subpath>,
        attributes: IndexMap<String, String>,
    },
    /// Rename a path (updates its display id and, on export, its `id` attr).
    RenamePath { path: usize, name: String },
    /// Soft-delete a whole path.
    DeletePath { path: usize },

    /// Set (`value: Some`) or clear (`value: None`) one presentation attribute. Added paths
    /// edit their own `attributes`; imported paths accumulate a `style_override`.
    SetStyle {
        path: usize,
        key: String,
        value: Option<String>,
    },
}

/// Apply an op to the document in place. Returns `true` if it found its target and mutated,
/// `false` if it was a no-op (missing target / invalid) — leaving the document untouched.
pub fn apply(doc: &mut SvgDocument, op: &Op) -> bool {
    match op {
        Op::MoveNode { node, to } => {
            let Some(n) = node_mut(doc, *node) else {
                return false;
            };
            let dx = to.x - n.point.x;
            let dy = to.y - n.point.y;
            n.point = *to;
            offset_handles(n, dx, dy);
            mark_edited(doc, node.path_index);
            true
        }
        Op::MoveHandle { node, which, to } => {
            let Some(n) = node_mut(doc, *node) else {
                return false;
            };
            set_handle(n, *which, Some(*to));
            if n.node_type == NodeType::Smooth {
                let opposite = handle(n, which.opposite());
                if let Some(opp) = opposite {
                    let len = distance(n.point, opp);
                    let dir = normalize(Point::new(n.point.x - to.x, n.point.y - to.y));
                    let mirrored = Point::new(n.point.x + dir.x * len, n.point.y + dir.y * len);
                    set_handle(n, which.opposite(), Some(mirrored));
                }
            }
            mark_edited(doc, node.path_index);
            true
        }
        Op::SetNodeType { node, node_type } => {
            let Some(n) = node_mut(doc, *node) else {
                return false;
            };
            n.node_type = *node_type;
            mark_edited(doc, node.path_index);
            true
        }
        Op::SetPenHandles { node, out } => {
            let Some(n) = node_mut(doc, *node) else {
                return false;
            };
            n.handle_out = Some(*out);
            n.handle_in = Some(Point::new(2.0 * n.point.x - out.x, 2.0 * n.point.y - out.y));
            n.node_type = NodeType::Smooth;
            mark_edited(doc, node.path_index);
            true
        }
        Op::SetSubpaths { path, subpaths } => {
            let Some(p) = doc.paths.get_mut(*path) else {
                return false;
            };
            p.subpaths = subpaths.clone();
            p.edited = true;
            true
        }
        Op::MovePathBy { path, dx, dy } => {
            let Some(p) = doc.paths.get_mut(*path) else {
                return false;
            };
            for sp in &mut p.subpaths {
                for n in &mut sp.nodes {
                    n.point.x += dx;
                    n.point.y += dy;
                    offset_handles(n, *dx, *dy);
                }
            }
            p.edited = true;
            true
        }
        Op::InsertNode {
            path,
            subpath,
            segment,
            t,
        } => {
            let Some(sp) = subpath_mut(doc, *path, *subpath) else {
                return false;
            };
            insert_node_at(sp, *segment, *t);
            mark_edited(doc, *path);
            true
        }
        Op::DeleteNode { node } => {
            let Some(path) = doc.paths.get_mut(node.path_index) else {
                return false;
            };
            let Some(sp) = path.subpaths.get_mut(node.subpath_index) else {
                return false;
            };
            if node.node_index >= sp.nodes.len() {
                return false;
            }
            sp.nodes.remove(node.node_index);
            let too_short = sp.nodes.len() < 2;
            if too_short {
                path.subpaths.remove(node.subpath_index);
            }
            if path.subpaths.is_empty() {
                path.deleted = true;
            }
            path.edited = true;
            true
        }
        Op::AppendNode {
            path,
            subpath,
            point,
        } => {
            let Some(sp) = subpath_mut(doc, *path, *subpath) else {
                return false;
            };
            sp.nodes.push(PathNode::corner(*point));
            mark_edited(doc, *path);
            true
        }
        Op::ReverseSubpath { path, subpath } => {
            let Some(sp) = subpath_mut(doc, *path, *subpath) else {
                return false;
            };
            *sp = reversed_subpath(sp);
            mark_edited(doc, *path);
            true
        }
        Op::CloseLoop { path, subpath } => {
            let Some(sp) = subpath_mut(doc, *path, *subpath) else {
                return false;
            };
            if sp.closed || sp.nodes.len() < 2 {
                return false;
            }
            let first = sp.nodes[0].point;
            if let Some(last) = sp.nodes.last_mut() {
                last.point = first;
            }
            close_subpath(sp);
            mark_edited(doc, *path);
            true
        }
        Op::ClosePath { path, subpath } => {
            let Some(sp) = subpath_mut(doc, *path, *subpath) else {
                return false;
            };
            if sp.closed || sp.nodes.len() < 2 {
                return false;
            }
            close_subpath(sp);
            mark_edited(doc, *path);
            true
        }
        Op::ResizeEllipse {
            path,
            subpath,
            center,
            rx,
            ry,
        } => {
            let Some(sp) = subpath_mut(doc, *path, *subpath) else {
                return false;
            };
            sp.nodes = ellipse_nodes(center.x, center.y, *rx, *ry);
            sp.closed = true;
            mark_edited(doc, *path);
            true
        }
        Op::AddPath {
            id,
            subpaths,
            attributes,
        } => {
            let index = doc.paths.len();
            doc.paths.push(PathElement {
                id: id.clone(),
                index,
                original_d: String::new(),
                subpaths: subpaths.clone(),
                edited: true,
                added: true,
                attributes: Some(attributes.clone()),
                style_override: None,
                original_tag: None,
                deleted: false,
                renamed: false,
            });
            true
        }
        Op::RenamePath { path, name } => {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return false;
            }
            let Some(p) = doc.paths.get_mut(*path) else {
                return false;
            };
            p.id = trimmed.to_string();
            p.renamed = true;
            true
        }
        Op::DeletePath { path } => {
            let Some(p) = doc.paths.get_mut(*path) else {
                return false;
            };
            p.deleted = true;
            true
        }
        Op::SetStyle { path, key, value } => {
            let Some(p) = doc.paths.get_mut(*path) else {
                return false;
            };
            let map = if p.added {
                p.attributes.get_or_insert_with(IndexMap::new)
            } else {
                p.style_override.get_or_insert_with(IndexMap::new)
            };
            match value {
                Some(v) => {
                    map.insert(key.clone(), v.clone());
                }
                None => {
                    map.shift_remove(key);
                }
            }
            true
        }
    }
}

impl Handle {
    fn opposite(self) -> Handle {
        match self {
            Handle::In => Handle::Out,
            Handle::Out => Handle::In,
        }
    }
}

fn handle(n: &PathNode, which: Handle) -> Option<Point> {
    match which {
        Handle::In => n.handle_in,
        Handle::Out => n.handle_out,
    }
}

fn set_handle(n: &mut PathNode, which: Handle, value: Option<Point>) {
    match which {
        Handle::In => n.handle_in = value,
        Handle::Out => n.handle_out = value,
    }
}

fn offset_handles(n: &mut PathNode, dx: f64, dy: f64) {
    if let Some(h) = n.handle_in.as_mut() {
        h.x += dx;
        h.y += dy;
    }
    if let Some(h) = n.handle_out.as_mut() {
        h.x += dx;
        h.y += dy;
    }
}

fn node_mut(doc: &mut SvgDocument, r: NodeRef) -> Option<&mut PathNode> {
    doc.paths
        .get_mut(r.path_index)?
        .subpaths
        .get_mut(r.subpath_index)?
        .nodes
        .get_mut(r.node_index)
}

fn subpath_mut(doc: &mut SvgDocument, path: usize, subpath: usize) -> Option<&mut Subpath> {
    doc.paths.get_mut(path)?.subpaths.get_mut(subpath)
}

fn mark_edited(doc: &mut SvgDocument, path: usize) {
    if let Some(p) = doc.paths.get_mut(path) {
        p.edited = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::path::parse_path_d;
    use crate::model::types::ViewBox;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    fn nref(path: usize, subpath: usize, node: usize) -> NodeRef {
        NodeRef {
            path_index: path,
            subpath_index: subpath,
            node_index: node,
        }
    }

    /// A one-path document from a `d` string, `added` flag controlling style routing.
    fn doc_from(d: &str, added: bool) -> SvgDocument {
        SvgDocument {
            source: String::new(),
            view_box: ViewBox {
                min_x: 0.0,
                min_y: 0.0,
                width: 100.0,
                height: 100.0,
            },
            paths: vec![PathElement {
                id: "p0".into(),
                index: 0,
                original_d: d.to_string(),
                subpaths: parse_path_d(d),
                edited: false,
                added,
                attributes: Some(IndexMap::new()),
                style_override: None,
                original_tag: None,
                deleted: false,
                renamed: false,
            }],
        }
    }

    #[test]
    fn move_node_carries_handles_and_marks_edited() {
        let mut doc = doc_from("M 0 0 C 0 10 10 10 10 0", true);
        assert!(apply(
            &mut doc,
            &Op::MoveNode {
                node: nref(0, 0, 0),
                to: Point::new(5.0, 5.0),
            }
        ));
        let n = &doc.paths[0].subpaths[0].nodes[0];
        assert_eq!(n.point, Point::new(5.0, 5.0));
        assert_eq!(n.handle_out, Some(Point::new(5.0, 15.0)));
        assert!(doc.paths[0].edited);
    }

    #[test]
    fn move_handle_mirrors_the_opposite_on_a_smooth_node() {
        let mut doc = doc_from("M 0 0 L 20 0", true);
        // hand-craft a smooth node with both handles.
        doc.paths[0].subpaths[0].nodes[0] = PathNode {
            point: Point::new(5.0, 5.0),
            handle_in: Some(Point::new(5.0, 10.0)),
            handle_out: Some(Point::new(5.0, 0.0)),
            node_type: NodeType::Smooth,
        };
        assert!(apply(
            &mut doc,
            &Op::MoveHandle {
                node: nref(0, 0, 0),
                which: Handle::Out,
                to: Point::new(7.0, 5.0),
            }
        ));
        let n = &doc.paths[0].subpaths[0].nodes[0];
        assert_eq!(n.handle_out, Some(Point::new(7.0, 5.0)));
        let hi = n.handle_in.unwrap();
        assert!(
            close(hi.x, 0.0) && close(hi.y, 5.0),
            "mirrored handle: {hi:?}"
        );
    }

    #[test]
    fn move_path_by_translates_everything() {
        let mut doc = doc_from("M 0 0 C 0 10 10 10 10 0", true);
        assert!(apply(
            &mut doc,
            &Op::MovePathBy {
                path: 0,
                dx: 3.0,
                dy: -2.0
            }
        ));
        let sp = &doc.paths[0].subpaths[0];
        assert_eq!(sp.nodes[0].point, Point::new(3.0, -2.0));
        assert_eq!(sp.nodes[0].handle_out, Some(Point::new(3.0, 8.0)));
        assert_eq!(sp.nodes[1].point, Point::new(13.0, -2.0));
    }

    #[test]
    fn insert_and_delete_node() {
        let mut doc = doc_from("M 0 0 L 10 0 L 10 10", true);
        assert!(apply(
            &mut doc,
            &Op::InsertNode {
                path: 0,
                subpath: 0,
                segment: 0,
                t: 0.5,
            }
        ));
        assert_eq!(doc.paths[0].subpaths[0].nodes.len(), 4);
        assert!(apply(
            &mut doc,
            &Op::DeleteNode {
                node: nref(0, 0, 1)
            }
        ));
        assert_eq!(doc.paths[0].subpaths[0].nodes.len(), 3);
    }

    #[test]
    fn delete_node_soft_deletes_an_emptied_path() {
        // two nodes: deleting one leaves < 2, dropping the subpath → path emptied → deleted.
        let mut doc = doc_from("M 0 0 L 10 0", true);
        assert!(apply(
            &mut doc,
            &Op::DeleteNode {
                node: nref(0, 0, 0)
            }
        ));
        assert!(doc.paths[0].subpaths.is_empty());
        assert!(doc.paths[0].deleted);
    }

    #[test]
    fn set_style_routes_to_attributes_for_added_paths() {
        let mut doc = doc_from("M 0 0 L 10 0", true);
        apply(
            &mut doc,
            &Op::SetStyle {
                path: 0,
                key: "fill".into(),
                value: Some("red".into()),
            },
        );
        assert_eq!(
            doc.paths[0]
                .attributes
                .as_ref()
                .unwrap()
                .get("fill")
                .map(String::as_str),
            Some("red")
        );
        assert!(doc.paths[0].style_override.is_none());
        // clearing removes the key
        apply(
            &mut doc,
            &Op::SetStyle {
                path: 0,
                key: "fill".into(),
                value: None,
            },
        );
        assert!(
            !doc.paths[0]
                .attributes
                .as_ref()
                .unwrap()
                .contains_key("fill")
        );
    }

    #[test]
    fn set_style_routes_to_override_for_imported_paths() {
        let mut doc = doc_from("M 0 0 L 10 0", false);
        apply(
            &mut doc,
            &Op::SetStyle {
                path: 0,
                key: "stroke".into(),
                value: Some("blue".into()),
            },
        );
        assert_eq!(
            doc.paths[0]
                .style_override
                .as_ref()
                .unwrap()
                .get("stroke")
                .map(String::as_str),
            Some("blue")
        );
    }

    #[test]
    fn add_path_appends_a_drawn_path() {
        let mut doc = doc_from("M 0 0 L 10 0", false);
        let mut attrs = IndexMap::new();
        attrs.insert("fill".to_string(), "none".to_string());
        assert!(apply(
            &mut doc,
            &Op::AddPath {
                id: "drawn-1".into(),
                subpaths: parse_path_d("M 1 1 L 2 2"),
                attributes: attrs,
            }
        ));
        assert_eq!(doc.paths.len(), 2);
        let p = &doc.paths[1];
        assert_eq!(p.id, "drawn-1");
        assert_eq!(p.index, 1);
        assert!(p.added && p.edited);
    }

    #[test]
    fn rename_path_requires_a_non_blank_name() {
        let mut doc = doc_from("M 0 0 L 10 0", true);
        assert!(!apply(
            &mut doc,
            &Op::RenamePath {
                path: 0,
                name: "  ".into()
            }
        ));
        assert!(!doc.paths[0].renamed);
        assert!(apply(
            &mut doc,
            &Op::RenamePath {
                path: 0,
                name: " star ".into()
            }
        ));
        assert_eq!(doc.paths[0].id, "star");
        assert!(doc.paths[0].renamed);
    }

    #[test]
    fn close_path_and_close_loop() {
        let mut doc = doc_from("M 0 0 L 10 0 L 10 10", true);
        assert!(apply(
            &mut doc,
            &Op::ClosePath {
                path: 0,
                subpath: 0
            }
        ));
        assert!(doc.paths[0].subpaths[0].closed);

        let mut doc2 = doc_from("M 0 0 L 10 0 L 10 10", true);
        assert!(apply(
            &mut doc2,
            &Op::CloseLoop {
                path: 0,
                subpath: 0
            }
        ));
        assert!(doc2.paths[0].subpaths[0].closed);
    }

    #[test]
    fn resize_ellipse_rebuilds_four_smooth_nodes() {
        let mut doc = doc_from("M 0 0 L 10 0", true);
        assert!(apply(
            &mut doc,
            &Op::ResizeEllipse {
                path: 0,
                subpath: 0,
                center: Point::new(50.0, 50.0),
                rx: 10.0,
                ry: 20.0,
            }
        ));
        let sp = &doc.paths[0].subpaths[0];
        assert_eq!(sp.nodes.len(), 4);
        assert!(sp.closed);
        assert_eq!(sp.nodes[0].point, Point::new(60.0, 50.0));
    }

    #[test]
    fn set_pen_handles_makes_a_smooth_mirror() {
        let mut doc = doc_from("M 5 5 L 10 10", true);
        assert!(apply(
            &mut doc,
            &Op::SetPenHandles {
                node: nref(0, 0, 0),
                out: Point::new(8.0, 5.0),
            }
        ));
        let n = &doc.paths[0].subpaths[0].nodes[0];
        assert_eq!(n.handle_out, Some(Point::new(8.0, 5.0)));
        assert_eq!(n.handle_in, Some(Point::new(2.0, 5.0))); // 2*point - out
        assert_eq!(n.node_type, NodeType::Smooth);
    }

    #[test]
    fn missing_target_is_a_no_op() {
        let mut doc = doc_from("M 0 0 L 10 0", true);
        let before = doc.clone();
        assert!(!apply(
            &mut doc,
            &Op::MoveNode {
                node: nref(9, 0, 0),
                to: Point::new(1.0, 1.0),
            }
        ));
        assert_eq!(doc, before);
    }
}
