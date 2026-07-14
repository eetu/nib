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
use crate::model::shapes::{ellipse_nodes, line_nodes, polygon_nodes, rect_nodes, star_nodes};
use crate::model::types::{
    Gradient, Layer, NodeRef, NodeType, PathElement, PathNode, Point, Subpath, SvgDocument,
};

/// Which control handle of a node an op targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Handle {
    In,
    Out,
}

/// A parametric primitive. One vocabulary for every shape tool + the MCP surface ("make a
/// rect / star here"); the reducer builds the anchor nodes from it, so shapes stay ordinary
/// editable paths.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "camelCase")]
pub enum ShapeSpec {
    Ellipse { cx: f64, cy: f64, rx: f64, ry: f64 },
    Rect { x0: f64, y0: f64, x1: f64, y1: f64 },
    Line { x0: f64, y0: f64, x1: f64, y1: f64 },
    Polygon { cx: f64, cy: f64, r: f64, sides: u32, rotation: f64 },
    Star { cx: f64, cy: f64, outer: f64, inner: f64, points: u32, rotation: f64 },
}

impl ShapeSpec {
    /// Build the shape's anchor nodes + whether the subpath is closed (lines are open).
    fn build(&self) -> (Vec<PathNode>, bool) {
        match *self {
            ShapeSpec::Ellipse { cx, cy, rx, ry } => (ellipse_nodes(cx, cy, rx, ry), true),
            ShapeSpec::Rect { x0, y0, x1, y1 } => (rect_nodes(x0, y0, x1, y1), true),
            ShapeSpec::Line { x0, y0, x1, y1 } => (line_nodes(x0, y0, x1, y1), false),
            ShapeSpec::Polygon { cx, cy, r, sides, rotation } => {
                (polygon_nodes(cx, cy, r, sides, rotation), true)
            }
            ShapeSpec::Star { cx, cy, outer, inner, points, rotation } => {
                (star_nodes(cx, cy, outer, inner, points, rotation), true)
            }
        }
    }
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
    SetNodeType {
        node: NodeRef,
        #[serde(rename = "nodeType")]
        node_type: NodeType,
    },
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
    /// Rebuild a subpath from a parametric shape spec (shape tools resize live through this).
    SetShape {
        path: usize,
        subpath: usize,
        spec: ShapeSpec,
    },

    /// Append a new drawn path (pen start, paste). The caller supplies the id (so all clients
    /// agree) and the resolved style.
    AddPath {
        id: String,
        subpaths: Vec<Subpath>,
        attributes: IndexMap<String, String>,
    },
    /// Append a new drawn path built from a shape spec (shape tools + MCP).
    AddShape {
        id: String,
        spec: ShapeSpec,
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

    /// Create a new layer (client supplies the id) and make it the active layer.
    AddLayer { id: String, name: String },
    /// Rename a layer.
    RenameLayer { id: String, name: String },
    /// Remove a layer; its paths become unassigned (the layer's contents are not deleted).
    DeleteLayer { id: String },
    /// Show or hide a layer (hidden layers omit their paths from render + export).
    SetLayerVisible { id: String, visible: bool },
    /// Move a layer to a new z-index in the ordered layers list.
    ReorderLayer { id: String, to: usize },
    /// Set (or clear, with `None`) the active layer new shapes are added to.
    SetActiveLayer {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        id: Option<String>,
    },
    /// Assign a path to a layer (`None` = unassign).
    SetPathLayer {
        path: usize,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        layer: Option<String>,
    },

    /// Upsert a gradient paint (matched by id) into the document's defs.
    SetGradient { gradient: Gradient },
    /// Remove a gradient by id.
    RemoveGradient { id: String },
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
        Op::SetShape {
            path,
            subpath,
            spec,
        } => {
            let Some(sp) = subpath_mut(doc, *path, *subpath) else {
                return false;
            };
            let (nodes, closed) = spec.build();
            sp.nodes = nodes;
            sp.closed = closed;
            mark_edited(doc, *path);
            true
        }
        Op::AddPath {
            id,
            subpaths,
            attributes,
        } => {
            let index = doc.paths.len();
            let layer = doc.active_layer.clone();
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
                layer,
            });
            true
        }
        Op::AddShape {
            id,
            spec,
            attributes,
        } => {
            let (nodes, closed) = spec.build();
            let index = doc.paths.len();
            let layer = doc.active_layer.clone();
            doc.paths.push(PathElement {
                id: id.clone(),
                index,
                original_d: String::new(),
                subpaths: vec![Subpath { nodes, closed }],
                edited: true,
                added: true,
                attributes: Some(attributes.clone()),
                style_override: None,
                original_tag: None,
                deleted: false,
                renamed: false,
                layer,
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

        Op::AddLayer { id, name } => {
            if doc.layers.iter().any(|l| &l.id == id) {
                return false;
            }
            doc.layers.push(Layer {
                id: id.clone(),
                name: name.clone(),
                visible: true,
            });
            doc.active_layer = Some(id.clone());
            true
        }
        Op::RenameLayer { id, name } => {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return false;
            }
            let Some(l) = doc.layers.iter_mut().find(|l| &l.id == id) else {
                return false;
            };
            l.name = trimmed.to_string();
            true
        }
        Op::DeleteLayer { id } => {
            let before = doc.layers.len();
            doc.layers.retain(|l| &l.id != id);
            if doc.layers.len() == before {
                return false;
            }
            for p in doc.paths.iter_mut() {
                if p.layer.as_deref() == Some(id.as_str()) {
                    p.layer = None;
                }
            }
            if doc.active_layer.as_deref() == Some(id.as_str()) {
                doc.active_layer = None;
            }
            true
        }
        Op::SetLayerVisible { id, visible } => {
            let Some(l) = doc.layers.iter_mut().find(|l| &l.id == id) else {
                return false;
            };
            if l.visible == *visible {
                return false;
            }
            l.visible = *visible;
            true
        }
        Op::ReorderLayer { id, to } => {
            let Some(from) = doc.layers.iter().position(|l| &l.id == id) else {
                return false;
            };
            let to = (*to).min(doc.layers.len().saturating_sub(1));
            if from == to {
                return false;
            }
            let l = doc.layers.remove(from);
            doc.layers.insert(to, l);
            true
        }
        Op::SetActiveLayer { id } => {
            if let Some(id) = id
                && !doc.layers.iter().any(|l| &l.id == id)
            {
                return false;
            }
            if doc.active_layer == *id {
                return false;
            }
            doc.active_layer = id.clone();
            true
        }
        Op::SetPathLayer { path, layer } => {
            if let Some(id) = layer
                && !doc.layers.iter().any(|l| &l.id == id)
            {
                return false;
            }
            let Some(p) = doc.paths.get_mut(*path) else {
                return false;
            };
            if p.layer == *layer {
                return false;
            }
            p.layer = layer.clone();
            true
        }

        Op::SetGradient { gradient } => {
            if let Some(g) = doc.gradients.iter_mut().find(|g| g.id == gradient.id) {
                if g == gradient {
                    return false;
                }
                *g = gradient.clone();
            } else {
                doc.gradients.push(gradient.clone());
            }
            true
        }
        Op::RemoveGradient { id } => {
            let before = doc.gradients.len();
            doc.gradients.retain(|g| &g.id != id);
            doc.gradients.len() != before
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
                layer: None,
            }],
            layers: Vec::new(),
            active_layer: None,
            gradients: Vec::new(),
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
        assert_eq!(p.layer, None); // no active layer → unassigned
    }

    #[test]
    fn layer_lifecycle_and_active_assignment() {
        let mut doc = doc_from("M 0 0 L 10 0", false);
        // Add a layer → it becomes active.
        assert!(apply(
            &mut doc,
            &Op::AddLayer {
                id: "L1".into(),
                name: "shapes".into()
            }
        ));
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.active_layer.as_deref(), Some("L1"));
        // A duplicate id is a no-op.
        assert!(!apply(
            &mut doc,
            &Op::AddLayer {
                id: "L1".into(),
                name: "dupe".into()
            }
        ));
        // A new drawn shape lands on the active layer.
        assert!(apply(
            &mut doc,
            &Op::AddPath {
                id: "s1".into(),
                subpaths: parse_path_d("M 1 1 L 2 2"),
                attributes: IndexMap::new(),
            }
        ));
        assert_eq!(doc.paths[1].layer.as_deref(), Some("L1"));
        // Hide + rename.
        assert!(apply(
            &mut doc,
            &Op::SetLayerVisible {
                id: "L1".into(),
                visible: false
            }
        ));
        assert!(!doc.layers[0].visible);
        assert!(apply(
            &mut doc,
            &Op::RenameLayer {
                id: "L1".into(),
                name: "outline".into()
            }
        ));
        assert_eq!(doc.layers[0].name, "outline");
        // Deleting the layer unassigns its paths + clears active.
        assert!(apply(&mut doc, &Op::DeleteLayer { id: "L1".into() }));
        assert!(doc.layers.is_empty());
        assert_eq!(doc.paths[1].layer, None);
        assert_eq!(doc.active_layer, None);
    }

    #[test]
    fn reorder_layer_moves_within_the_z_order() {
        let mut doc = doc_from("M 0 0 L 10 0", false);
        for id in ["a", "b", "c"] {
            apply(
                &mut doc,
                &Op::AddLayer {
                    id: id.into(),
                    name: id.into(),
                },
            );
        }
        // a,b,c → move c to front (index 0)
        assert!(apply(
            &mut doc,
            &Op::ReorderLayer {
                id: "c".into(),
                to: 0
            }
        ));
        let order: Vec<&str> = doc.layers.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(order, ["c", "a", "b"]);
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
    fn set_shape_rebuilds_an_ellipse() {
        let mut doc = doc_from("M 0 0 L 10 0", true);
        assert!(apply(
            &mut doc,
            &Op::SetShape {
                path: 0,
                subpath: 0,
                spec: ShapeSpec::Ellipse {
                    cx: 50.0,
                    cy: 50.0,
                    rx: 10.0,
                    ry: 20.0,
                },
            }
        ));
        let sp = &doc.paths[0].subpaths[0];
        assert_eq!(sp.nodes.len(), 4);
        assert!(sp.closed);
        assert_eq!(sp.nodes[0].point, Point::new(60.0, 50.0));
    }

    #[test]
    fn set_shape_builds_a_rect_and_a_line() {
        let mut doc = doc_from("M 0 0 L 10 0", true);
        apply(
            &mut doc,
            &Op::SetShape {
                path: 0,
                subpath: 0,
                spec: ShapeSpec::Rect { x0: 10.0, y0: 20.0, x1: 30.0, y1: 40.0 },
            },
        );
        assert_eq!(doc.paths[0].subpaths[0].nodes.len(), 4);
        assert!(doc.paths[0].subpaths[0].closed);

        apply(
            &mut doc,
            &Op::SetShape {
                path: 0,
                subpath: 0,
                spec: ShapeSpec::Line { x0: 0.0, y0: 0.0, x1: 5.0, y1: 5.0 },
            },
        );
        assert_eq!(doc.paths[0].subpaths[0].nodes.len(), 2);
        assert!(!doc.paths[0].subpaths[0].closed); // lines are open
    }

    #[test]
    fn add_shape_appends_a_polygon_path() {
        let mut doc = doc_from("M 0 0 L 10 0", false);
        assert!(apply(
            &mut doc,
            &Op::AddShape {
                id: "poly".into(),
                spec: ShapeSpec::Polygon {
                    cx: 0.0,
                    cy: 0.0,
                    r: 10.0,
                    sides: 6,
                    rotation: 0.0,
                },
                attributes: IndexMap::new(),
            }
        ));
        assert_eq!(doc.paths.len(), 2);
        assert_eq!(doc.paths[1].subpaths[0].nodes.len(), 6);
        assert!(doc.paths[1].added);
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
