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
    Gradient, NodeRef, NodeType, PathElement, PathNode, Point, Subpath, SvgDocument,
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
    Ellipse {
        cx: f64,
        cy: f64,
        rx: f64,
        ry: f64,
    },
    Rect {
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
    },
    Line {
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
    },
    Polygon {
        cx: f64,
        cy: f64,
        r: f64,
        sides: u32,
        rotation: f64,
    },
    Star {
        cx: f64,
        cy: f64,
        outer: f64,
        inner: f64,
        points: u32,
        rotation: f64,
    },
}

impl ShapeSpec {
    /// Build the shape's anchor nodes + whether the subpath is closed (lines are open).
    fn build(&self) -> (Vec<PathNode>, bool) {
        match *self {
            ShapeSpec::Ellipse { cx, cy, rx, ry } => (ellipse_nodes(cx, cy, rx, ry), true),
            ShapeSpec::Rect { x0, y0, x1, y1 } => (rect_nodes(x0, y0, x1, y1), true),
            ShapeSpec::Line { x0, y0, x1, y1 } => (line_nodes(x0, y0, x1, y1), false),
            ShapeSpec::Polygon {
                cx,
                cy,
                r,
                sides,
                rotation,
            } => (polygon_nodes(cx, cy, r, sides, rotation), true),
            ShapeSpec::Star {
                cx,
                cy,
                outer,
                inner,
                points,
                rotation,
            } => (star_nodes(cx, cy, outer, inner, points, rotation), true),
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
    /// Move a path within the ordered paths list — changes draw order (later = drawn on top).
    ReorderPath { from: usize, to: usize },
    /// Show/hide a single path.
    SetPathHidden { path: usize, hidden: bool },
    /// Show/hide any node in the document **tree** by its stable `uid` — a group, an opaque
    /// element, or a shape, at any depth (structural op). Exports as `display="none"`, skipped in
    /// the render. The tree is the structural model; `SetPathHidden` is the per-path sibling.
    SetNodeHidden { uid: String, hidden: bool },
    /// Wrap tree nodes (`uids`, which must share one parent) in a new `<g uid id="name">` at the
    /// first member's slot — the nested-group op on the object tree. `uid` = the new group's
    /// stable id; `name` = its `id` attribute. Re-projects the paths view (order may change).
    GroupNodes {
        uids: Vec<String>,
        uid: String,
        name: String,
    },
    /// Dissolve a group node (`uid`) in the tree, splicing its children into its parent in place.
    UngroupNode { uid: String },
    /// Move a tree node one element-slot within its parent — `forward` (later in document order =
    /// higher z) or backward. Re-projects the paths view (z-order changed).
    ReorderNode { uid: String, forward: bool },
    /// Move a tree node relative to `ref_uid` — `position` = "before"/"after" (sibling, reparenting
    /// across levels) or "inside" (into a group). The drag-drop reorder op. Re-projects the paths.
    /// (Distinct from `MoveNode`, which moves an anchor *point*.)
    MoveTreeNode {
        uid: String,
        #[serde(rename = "refUid")]
        ref_uid: String,
        position: String,
    },
    /// Set (`Some`) or clear (`None`) the live-boolean op on a group node (by `uid`) in the tree:
    /// turn a plain `<g>` into a live boolean, change the operation, or flatten it back. The
    /// group's element children are the operands; the tree renders/exports the computed result.
    SetNodeBoolean {
        uid: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        op: Option<String>,
    },
    /// Set (`Some`) or remove (`None`) one attribute on ANY tree node by `uid` — the generic
    /// attribute editor for non-shape elements (text/image/use/…): geometry (x/y/width/height),
    /// a `transform` matrix (drag/scale/rotate), or presentation (fill/font-size/…). Marks the
    /// node edited so it regenerates on emit.
    SetNodeAttr {
        uid: String,
        key: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        value: Option<String>,
    },
    /// Replace a text-bearing element's content (its child text) — editing a `<text>` label.
    SetNodeText { uid: String, text: String },

    /// Set (`value: Some`) or clear (`value: None`) one presentation attribute. Added paths
    /// edit their own `attributes`; imported paths accumulate a `style_override`.
    SetStyle {
        path: usize,
        key: String,
        value: Option<String>,
    },

    /// Upsert a gradient paint (matched by id) into the document's defs.
    SetGradient { gradient: Gradient },
    /// Remove a gradient by id.
    RemoveGradient { id: String },

    /// Combine paths with a boolean op ("union"|"intersect"|"subtract"|"exclude"): the inputs
    /// are soft-deleted and replaced by one new path (`id`) built from the result. Curves are
    /// flattened to lines. For "subtract", the lowest-index path is the subject.
    BooleanOp {
        op: String,
        paths: Vec<usize>,
        id: String,
    },
    /// Merge paths into one **compound path** (`id`) holding all their subpaths in draw order —
    /// the inputs are soft-deleted. Unlike `BooleanOp` this keeps the subpaths distinct (no
    /// geometry merge), so a line + a detached dome become one editable element. Inherits the
    /// style + group of the backmost member that actually fills (so a filled shape + a
    /// stroke-only one keeps the fill, not the stroke-only member's `fill="none"`).
    CombinePaths { paths: Vec<usize>, id: String },
    /// Release a compound path: soft-delete it and add one path per subpath (`ids`, one per
    /// subpath) so each becomes independently styleable. Inherits the source's style + group.
    ReleaseCompound { path: usize, ids: Vec<String> },
    /// Reduce a path's node count (Ramer–Douglas–Peucker) within `tolerance` document units.
    SimplifyPath { path: usize, tolerance: f64 },
    /// Expand a path's stroke (`width`) into a filled outline: the source is soft-deleted and a
    /// new fill path (`id`) is added, its fill taken from the source's stroke colour.
    OutlineStroke { path: usize, width: f64, id: String },
    /// Offset a path's outline by `distance` (outward if positive, inward if negative), adding
    /// the result as a new path (`id`) that inherits the source's style; the source is kept.
    OffsetPath {
        path: usize,
        distance: f64,
        id: String,
    },
}

/// A fresh in-app-drawn `PathElement` (its `uid`/`index` are filled in by [`add_drawn`]). The
/// standard shape for every added path — pen/shape/paste plus boolean/outline/offset results.
fn drawn_path(id: String, subpaths: Vec<Subpath>, attributes: IndexMap<String, String>) -> PathElement {
    PathElement {
        id,
        uid: String::new(),
        index: 0,
        original_d: String::new(),
        subpaths,
        edited: true,
        added: true,
        attributes: Some(attributes),
        style_override: None,
        original_tag: None,
        deleted: false,
        renamed: false,
        hidden: false,
    }
}

/// The single funnel for adding a drawn path: mint a tree `uid`, append a `<path>` node to the
/// tree (top of z-order), and push the flat `PathElement`. Drawn content thus lives in the tree
/// exactly like imported content — one representation, one serializer, unified z-order + panel.
/// Returns the new path's index in `doc.paths`.
fn add_drawn(doc: &mut SvgDocument, mut path: PathElement) -> usize {
    let index = doc.paths.len();
    path.index = index;
    path.added = true;
    if let Some(tree) = doc.tree.as_mut() {
        let uid = tree.fresh_uid();
        path.uid = uid.clone();
        let attrs = path.attributes.clone().unwrap_or_default();
        let d = crate::model::path::path_to_d_prec(&path.subpaths, 3);
        tree.append_drawn(crate::model::tree::make_path_node(&uid, &attrs, &d));
    }
    doc.paths.push(path);
    index
}

/// Ensure every drawn (`added`) path has a matching tree node — self-heals a document whose flat
/// `paths` carry added paths the tree lacks (a session persisted before drawn content moved into
/// the tree, or a `set_tree` that failed). Appends a `<path>` node per orphaned added path.
pub fn ensure_drawn_in_tree(doc: &mut SvgDocument) {
    let Some(tree) = doc.tree.as_mut() else {
        return;
    };
    let mut known: std::collections::HashSet<String> = tree
        .project_paths()
        .into_iter()
        .filter(|p| !p.uid.is_empty())
        .map(|p| p.uid)
        .collect();
    for p in doc.paths.iter_mut() {
        if !p.added || p.deleted {
            continue;
        }
        if !p.uid.is_empty() && known.contains(&p.uid) {
            continue;
        }
        let uid = tree.fresh_uid();
        p.uid = uid.clone();
        known.insert(uid.clone());
        let attrs = p.attributes.clone().unwrap_or_default();
        let d = crate::model::path::path_to_d_prec(&p.subpaths, 3);
        tree.append_drawn(crate::model::tree::make_path_node(&uid, &attrs, &d));
    }
}

/// Re-project the flat `paths` view from the (structurally-mutated) tree, **preserving edits**:
/// a structural op (group/ungroup/reorder) can change shape document order, so paths are rebuilt
/// from the tree, then each path's geometry/style/rename/hidden/delete/layer is carried over from
/// the old paths by `uid`. Drawn paths now live in the tree too (added via [`add_drawn`]), so they
/// project like imported ones; any stray uid-less path is re-appended defensively. Indices are
/// renumbered to array position. Keeps `doc.paths` consistent with the tree after structure edits
/// without discarding the user's geometry work.
fn reproject_paths(doc: &mut SvgDocument) {
    let Some(tree) = &doc.tree else {
        return;
    };
    let old: std::collections::HashMap<String, PathElement> = doc
        .paths
        .iter()
        .filter(|p| !p.uid.is_empty())
        .map(|p| (p.uid.clone(), p.clone()))
        .collect();
    let added: Vec<PathElement> = doc
        .paths
        .iter()
        .filter(|p| p.uid.is_empty())
        .cloned()
        .collect();
    let mut fresh = tree.project_paths();
    for p in fresh.iter_mut() {
        if let Some(o) = old.get(&p.uid) {
            p.subpaths = o.subpaths.clone();
            p.edited = o.edited;
            p.attributes = o.attributes.clone();
            p.style_override = o.style_override.clone();
            p.id = o.id.clone();
            p.renamed = o.renamed;
            p.deleted = o.deleted;
            p.hidden = o.hidden;
        }
    }
    fresh.extend(added);
    for (i, p) in fresh.iter_mut().enumerate() {
        p.index = i;
    }
    doc.paths = fresh;
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
            let Some(sp) = subpath_mut(doc, node.path_index, node.subpath_index) else {
                return false;
            };
            let ni = node.node_index;
            let count = sp.nodes.len();
            if ni >= count {
                return false;
            }
            sp.nodes[ni].node_type = *node_type;
            // Converting a handle-less node to smooth synthesizes a tangent from its neighbours
            // (Catmull-Rom style: direction = prev→next, each handle ~1/3 the neighbour gap) —
            // so it gains draggable control handles instead of staying a hard corner.
            let bare = sp.nodes[ni].handle_in.is_none() && sp.nodes[ni].handle_out.is_none();
            if *node_type == NodeType::Smooth && bare {
                let p = sp.nodes[ni].point;
                let prev = if ni > 0 {
                    Some(sp.nodes[ni - 1].point)
                } else if sp.closed {
                    Some(sp.nodes[count - 1].point)
                } else {
                    None
                };
                let next = if ni + 1 < count {
                    Some(sp.nodes[ni + 1].point)
                } else if sp.closed {
                    Some(sp.nodes[0].point)
                } else {
                    None
                };
                let dir = match (prev, next) {
                    (Some(a), Some(b)) => normalize(Point::new(b.x - a.x, b.y - a.y)),
                    (None, Some(b)) => normalize(Point::new(b.x - p.x, b.y - p.y)),
                    (Some(a), None) => normalize(Point::new(p.x - a.x, p.y - a.y)),
                    (None, None) => Point::new(0.0, 0.0),
                };
                let out_len = next.or(prev).map(|q| distance(p, q) / 3.0).unwrap_or(0.0);
                let in_len = prev.or(next).map(|q| distance(p, q) / 3.0).unwrap_or(0.0);
                sp.nodes[ni].handle_in =
                    Some(Point::new(p.x - dir.x * in_len, p.y - dir.y * in_len));
                sp.nodes[ni].handle_out =
                    Some(Point::new(p.x + dir.x * out_len, p.y + dir.y * out_len));
            }
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
            add_drawn(doc, drawn_path(id.clone(), subpaths.clone(), attributes.clone()));
            true
        }
        Op::AddShape {
            id,
            spec,
            attributes,
        } => {
            let (nodes, closed) = spec.build();
            add_drawn(
                doc,
                drawn_path(
                    id.clone(),
                    vec![Subpath { nodes, closed }],
                    attributes.clone(),
                ),
            );
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
        Op::ReorderPath { from, to } => {
            if *from >= doc.paths.len() {
                return false;
            }
            let to = (*to).min(doc.paths.len() - 1);
            if *from == to {
                return false;
            }
            let p = doc.paths.remove(*from);
            doc.paths.insert(to, p);
            true
        }
        Op::SetPathHidden { path, hidden } => {
            let Some(p) = doc.paths.get_mut(*path) else {
                return false;
            };
            if p.hidden == *hidden {
                return false;
            }
            p.hidden = *hidden;
            true
        }
        Op::SetNodeHidden { uid, hidden } => doc
            .tree
            .as_mut()
            .map(|t| t.set_hidden(uid, *hidden))
            .unwrap_or(false),
        Op::GroupNodes { uids, uid, name } => {
            let ok = doc
                .tree
                .as_mut()
                .map(|t| t.group(uids, uid, name))
                .unwrap_or(false);
            if ok {
                reproject_paths(doc); // wrapping may reorder shapes → refresh the paths view
            }
            ok
        }
        Op::UngroupNode { uid } => {
            let ok = doc.tree.as_mut().map(|t| t.ungroup(uid)).unwrap_or(false);
            if ok {
                reproject_paths(doc);
            }
            ok
        }
        Op::ReorderNode { uid, forward } => {
            let ok = doc
                .tree
                .as_mut()
                .map(|t| t.reorder(uid, *forward))
                .unwrap_or(false);
            if ok {
                reproject_paths(doc);
            }
            ok
        }
        Op::MoveTreeNode {
            uid,
            ref_uid,
            position,
        } => {
            let ok = doc
                .tree
                .as_mut()
                .map(|t| t.move_node(uid, ref_uid, position))
                .unwrap_or(false);
            if ok {
                reproject_paths(doc);
            }
            ok
        }
        Op::SetNodeBoolean { uid, op } => doc
            .tree
            .as_mut()
            .map(|t| t.set_boolean(uid, op.clone()))
            .unwrap_or(false),
        Op::SetNodeAttr { uid, key, value } => doc
            .tree
            .as_mut()
            .map(|t| t.set_node_attr(uid, key, value.as_deref()))
            .unwrap_or(false),
        Op::SetNodeText { uid, text } => doc
            .tree
            .as_mut()
            .map(|t| t.set_node_text(uid, text))
            .unwrap_or(false),
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
        Op::BooleanOp { op, paths, id } => {
            let mut idxs: Vec<usize> = paths
                .iter()
                .copied()
                .filter(|&i| doc.paths.get(i).is_some_and(|p| !p.deleted))
                .collect();
            idxs.sort_unstable();
            idxs.dedup();
            if idxs.len() < 2 {
                return false;
            }
            let subpaths = {
                let refs: Vec<&PathElement> = idxs.iter().map(|&i| &doc.paths[i]).collect();
                match crate::model::booleans::boolean(op, &refs) {
                    Some(s) => s,
                    None => return false,
                }
            };
            // The result inherits the subject's (lowest-index) effective style.
            let first = &doc.paths[idxs[0]];
            let mut attributes = first.attributes.clone().unwrap_or_default();
            if let Some(so) = &first.style_override {
                for (k, v) in so {
                    attributes.insert(k.clone(), v.clone());
                }
            }
            for &i in &idxs {
                doc.paths[i].deleted = true;
            }
            add_drawn(doc, drawn_path(id.clone(), subpaths, attributes));
            true
        }
        Op::CombinePaths { paths, id } => {
            let mut idxs: Vec<usize> = paths
                .iter()
                .copied()
                .filter(|&i| doc.paths.get(i).is_some_and(|p| !p.deleted))
                .collect();
            idxs.sort_unstable();
            idxs.dedup();
            if idxs.len() < 2 {
                return false;
            }
            let mut subpaths = Vec::new();
            for &i in &idxs {
                subpaths.extend(doc.paths[i].subpaths.iter().cloned());
            }
            if subpaths.is_empty() {
                return false;
            }
            // Base the compound's paint on the backmost member that actually fills — so
            // combining a filled shape with a stroke-only one (fill="none") keeps the fill
            // instead of inheriting the stroke-only member's no-fill. Falls back to backmost.
            let base = idxs
                .iter()
                .copied()
                .find(|&i| {
                    let p = &doc.paths[i];
                    let fill = p
                        .style_override
                        .as_ref()
                        .and_then(|s| s.get("fill"))
                        .or_else(|| p.attributes.as_ref().and_then(|a| a.get("fill")));
                    match fill {
                        Some(f) => f.as_str() != "none",
                        None => true, // no fill attr → SVG default is filled
                    }
                })
                .unwrap_or(idxs[0]);
            let first = &doc.paths[base];
            let mut attributes = first.attributes.clone().unwrap_or_default();
            if let Some(so) = &first.style_override {
                for (k, v) in so {
                    attributes.insert(k.clone(), v.clone());
                }
            }
            for &i in &idxs {
                doc.paths[i].deleted = true;
            }
            add_drawn(doc, drawn_path(id.clone(), subpaths, attributes));
            true
        }
        Op::ReleaseCompound { path, ids } => {
            let Some(p) = doc.paths.get(*path) else {
                return false;
            };
            if p.deleted || p.subpaths.len() < 2 {
                return false;
            }
            // Effective style flattens attributes ← styleOverride, so each released path
            // paints exactly as the compound did (they become `added` — no source slot).
            let mut attributes = p.attributes.clone().unwrap_or_default();
            if let Some(so) = &p.style_override {
                for (k, v) in so {
                    attributes.insert(k.clone(), v.clone());
                }
            }
            let base_id = p.id.clone();
            let subpaths = p.subpaths.clone();
            doc.paths[*path].deleted = true;
            for (k, sp) in subpaths.into_iter().enumerate() {
                let id = ids
                    .get(k)
                    .cloned()
                    .unwrap_or_else(|| format!("{}-{}", base_id, k + 1));
                add_drawn(doc, drawn_path(id, vec![sp], attributes.clone()));
            }
            true
        }
        Op::SimplifyPath { path, tolerance } => {
            let Some(p) = doc.paths.get_mut(*path) else {
                return false;
            };
            let before: usize = p.subpaths.iter().map(|sp| sp.nodes.len()).sum();
            let simplified = crate::model::path::simplify_subpaths(&p.subpaths, *tolerance);
            let after: usize = simplified.iter().map(|sp| sp.nodes.len()).sum();
            if after >= before {
                return false;
            }
            p.subpaths = simplified;
            p.edited = true;
            true
        }
        Op::OutlineStroke { path, width, id } => {
            let (subpaths, attributes) = {
                let Some(p) = doc.paths.get(*path) else {
                    return false;
                };
                if p.deleted {
                    return false;
                }
                let subpaths = crate::model::path::outline_stroke(&p.subpaths, *width, 0.25);
                if subpaths.is_empty() {
                    return false;
                }
                let mut m = p.attributes.clone().unwrap_or_default();
                if let Some(so) = &p.style_override {
                    for (k, v) in so {
                        m.insert(k.clone(), v.clone());
                    }
                }
                let stroke_color = m
                    .get("stroke")
                    .cloned()
                    .unwrap_or_else(|| "#000000".to_string());
                m.insert("fill".to_string(), stroke_color);
                m.insert("stroke".to_string(), "none".to_string());
                m.shift_remove("stroke-width");
                (subpaths, m)
            };
            doc.paths[*path].deleted = true;
            add_drawn(doc, drawn_path(id.clone(), subpaths, attributes));
            true
        }
        Op::OffsetPath { path, distance, id } => {
            let (subpaths, attributes) = {
                let Some(p) = doc.paths.get(*path) else {
                    return false;
                };
                if p.deleted {
                    return false;
                }
                let subpaths = match crate::model::booleans::offset_path(&p.subpaths, *distance) {
                    Some(s) => s,
                    None => return false,
                };
                let mut m = p.attributes.clone().unwrap_or_default();
                if let Some(so) = &p.style_override {
                    for (k, v) in so {
                        m.insert(k.clone(), v.clone());
                    }
                }
                (subpaths, m)
            };
            add_drawn(doc, drawn_path(id.clone(), subpaths, attributes));
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
                uid: String::new(),
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
                hidden: false,
            }],
            gradients: Vec::new(),
            tree: None,
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
    fn reorder_path_moves_within_the_draw_order() {
        let mut doc = doc_from("M 0 0 L 1 1", true);
        apply(
            &mut doc,
            &Op::AddPath {
                id: "b".into(),
                subpaths: parse_path_d("M 0 0 L 2 2"),
                attributes: IndexMap::new(),
            },
        );
        apply(
            &mut doc,
            &Op::AddPath {
                id: "c".into(),
                subpaths: parse_path_d("M 0 0 L 3 3"),
                attributes: IndexMap::new(),
            },
        );
        // p0, b, c → move c (index 2) to the front
        assert!(apply(&mut doc, &Op::ReorderPath { from: 2, to: 0 }));
        let ids: Vec<&str> = doc.paths.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["c", "p0", "b"]);
        // a no-op move returns false
        assert!(!apply(&mut doc, &Op::ReorderPath { from: 1, to: 1 }));
    }

    #[test]
    fn group_nodes_wraps_siblings_then_ungroup_dissolves() {
        use crate::model::document::{parse_svg, serialize_via_tree};
        let mut doc = parse_svg(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="0" y="0" width="10" height="10"/><rect x="20" y="0" width="10" height="10"/></svg>"#,
        )
        .unwrap();
        // Mimic the Editor's load: the projected paths (incl. primitives) are the working model.
        doc.paths = doc.tree.as_ref().unwrap().project_paths();
        let (u0, u1) = (doc.paths[0].uid.clone(), doc.paths[1].uid.clone());
        assert!(!u0.is_empty());

        // Group the two rects into <g id="grp">.
        assert!(apply(
            &mut doc,
            &Op::GroupNodes {
                uids: vec![u0.clone(), u1.clone()],
                uid: "grp-uid".into(),
                name: "grp".into(),
            }
        ));
        let tree = doc.tree.as_ref().unwrap();
        let out = serialize_via_tree(&doc, tree, 2);
        assert!(out.contains("<g id=\"grp\">"), "grouped: {out}");
        assert!(out.matches("<rect").count() == 2, "both rects kept: {out}");
        // paths view still has both, addressable by the same uids
        assert_eq!(doc.paths.len(), 2);
        assert!(doc.paths.iter().any(|p| p.uid == u0) && doc.paths.iter().any(|p| p.uid == u1));

        // Ungroup dissolves the <g>, rects back at top level.
        assert!(apply(
            &mut doc,
            &Op::UngroupNode {
                uid: "grp-uid".into()
            }
        ));
        let out2 = serialize_via_tree(&doc, doc.tree.as_ref().unwrap(), 2);
        assert!(!out2.contains("<g id=\"grp\""), "ungrouped: {out2}");
        assert!(
            out2.matches("<rect").count() == 2,
            "rects still there: {out2}"
        );
    }

    #[test]
    fn reorder_node_changes_document_z_order() {
        use crate::model::document::{parse_svg, serialize_via_tree};
        let mut doc = parse_svg(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect id="a" x="0" y="0" width="10" height="10"/><rect id="b" x="20" y="0" width="10" height="10"/></svg>"#,
        )
        .unwrap();
        doc.paths = doc.tree.as_ref().unwrap().project_paths();
        let ua = doc.paths[0].uid.clone(); // rect "a", first (lowest z)
        assert!(apply(
            &mut doc,
            &Op::ReorderNode {
                uid: ua,
                forward: true
            }
        ));
        let out = serialize_via_tree(&doc, doc.tree.as_ref().unwrap(), 2);
        let ia = out.find("id=\"a\"").unwrap();
        let ib = out.find("id=\"b\"").unwrap();
        assert!(ib < ia, "after bring-forward, b precedes a: {out}");
    }

    #[test]
    fn move_node_reorders_across_siblings_and_into_groups() {
        use crate::model::document::{parse_svg, serialize_via_tree};
        let mut doc = parse_svg(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect id="a" x="0" y="0" width="10" height="10"/><rect id="b" x="20" y="0" width="10" height="10"/><g id="grp"><rect id="c" x="40" y="0" width="10" height="10"/></g></svg>"#,
        )
        .unwrap();
        doc.paths = doc.tree.as_ref().unwrap().project_paths();
        fn uid_of(tree: &crate::model::tree::Tree, id: &str) -> String {
            fn walk<'a>(n: &'a crate::model::tree::Node, id: &str) -> Option<&'a str> {
                if let crate::model::tree::Node::Element {
                    attrs,
                    uid,
                    children,
                    ..
                } = n
                {
                    if attrs.iter().any(|(k, v)| k == "id" && v == id) {
                        return Some(uid);
                    }
                    for c in children {
                        if let Some(u) = walk(c, id) {
                            return Some(u);
                        }
                    }
                }
                None
            }
            walk(&tree.root, id).unwrap().to_string()
        }
        let t = doc.tree.as_ref().unwrap();
        let (ua, ub, ugrp, uc) = (uid_of(t, "a"), uid_of(t, "b"), uid_of(t, "grp"), uid_of(t, "c"));
        // Move `a` after `b` (reorder siblings) → document order b, a, <g>.
        assert!(apply(
            &mut doc,
            &Op::MoveTreeNode { uid: ua.clone(), ref_uid: ub, position: "after".into() }
        ));
        let out = serialize_via_tree(&doc, doc.tree.as_ref().unwrap(), 2);
        assert!(out.find("id=\"b\"").unwrap() < out.find("id=\"a\"").unwrap(), "b before a: {out}");
        // Move `a` INTO the group → it becomes a child of <g id="grp"> (reparent).
        assert!(apply(
            &mut doc,
            &Op::MoveTreeNode { uid: ua, ref_uid: ugrp.clone(), position: "inside".into() }
        ));
        let out2 = serialize_via_tree(&doc, doc.tree.as_ref().unwrap(), 2);
        // `a` now sits inside the group.
        let g = out2.find("id=\"grp\"").unwrap();
        let close = out2[g..].find("</g>").unwrap() + g;
        let a = out2.find("id=\"a\"").unwrap();
        assert!(a > g && a < close, "a moved inside the group: {out2}");
        // Cycle guard: moving the group inside its own child is a no-op.
        assert!(!apply(
            &mut doc,
            &Op::MoveTreeNode { uid: ugrp, ref_uid: uc, position: "inside".into() }
        ));
    }

    #[test]
    fn drawn_path_lives_in_the_tree_and_serializes_via_it() {
        use crate::model::document::{parse_svg, serialize_via_tree};
        let mut doc = parse_svg(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect id="a" x="0" y="0" width="10" height="10"/></svg>"#,
        )
        .unwrap();
        doc.paths = doc.tree.as_ref().unwrap().project_paths();
        let mut style = IndexMap::new();
        style.insert("fill".to_string(), "#f00".to_string());
        assert!(apply(
            &mut doc,
            &Op::AddPath {
                id: "drawn".into(),
                subpaths: parse_path_d("M 20 20 L 40 20 L 40 40 Z"),
                attributes: style,
            }
        ));
        // The drawn path got a tree uid + node (not a uid-less orphan).
        let last = doc.paths.last().unwrap();
        assert!(last.added && !last.uid.is_empty(), "drawn path is in the tree");
        // Serializing via the tree emits it (no separate append step) + keeps the rect verbatim.
        let out = serialize_via_tree(&doc, doc.tree.as_ref().unwrap(), 2);
        assert!(out.contains("M 20 20"), "drawn geometry emitted: {out}");
        assert!(out.contains("fill=\"#f00\""), "drawn style emitted: {out}");
        assert!(out.contains("<rect id=\"a\""), "imported rect preserved: {out}");
    }

    #[test]
    fn live_boolean_group_in_tree_bakes_and_computes_result() {
        use crate::model::document::{parse_svg, serialize_via_tree, tree_boolean_results};
        let mut doc =
            parse_svg(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"></svg>"#)
                .unwrap();
        doc.paths = doc.tree.as_ref().unwrap().project_paths();
        let rect = |x: f64| {
            let mut m = IndexMap::new();
            m.insert("fill".to_string(), "#00f".to_string());
            (
                parse_path_d(&format!(
                    "M {x} 0 L {} 0 L {} 20 L {x} 20 Z",
                    x + 30.0,
                    x + 30.0
                )),
                m,
            )
        };
        let (s0, a0) = rect(0.0);
        let (s1, a1) = rect(20.0);
        apply(
            &mut doc,
            &Op::AddPath {
                id: "r0".into(),
                subpaths: s0,
                attributes: a0,
            },
        );
        apply(
            &mut doc,
            &Op::AddPath {
                id: "r1".into(),
                subpaths: s1,
                attributes: a1,
            },
        );
        let (u0, u1) = (doc.paths[0].uid.clone(), doc.paths[1].uid.clone());
        assert!(apply(
            &mut doc,
            &Op::GroupNodes {
                uids: vec![u0, u1],
                uid: "grp".into(),
                name: "bool".into(),
            }
        ));
        assert!(apply(
            &mut doc,
            &Op::SetNodeBoolean {
                uid: "grp".into(),
                op: Some("subtract".into()),
            }
        ));
        // The live result is computed from the operands' geometry, keyed by the group uid.
        let results = tree_boolean_results(&doc);
        assert_eq!(results.len(), 1, "one live-boolean result");
        assert_eq!(results[0].uid, "grp");
        assert!(!results[0].subpaths.is_empty(), "non-empty subtract geometry");
        assert_eq!(results[0].operand_uids.len(), 2, "two operands recorded");
        // Export bakes the group to ONE computed path (operands not emitted separately).
        let out = serialize_via_tree(&doc, doc.tree.as_ref().unwrap(), 2);
        assert_eq!(
            out.matches("<path").count(),
            1,
            "boolean group bakes to one path: {out}"
        );
    }

    #[test]
    fn set_path_hidden_toggles() {
        let mut doc = doc_from("M 0 0 L 1 1", true);
        assert!(apply(
            &mut doc,
            &Op::SetPathHidden {
                path: 0,
                hidden: true
            }
        ));
        assert!(doc.paths[0].hidden);
        assert!(!apply(
            &mut doc,
            &Op::SetPathHidden {
                path: 0,
                hidden: true
            }
        ));
    }

    #[test]
    fn corner_to_smooth_synthesizes_a_tangent() {
        // 3 corners; converting the middle to smooth gains it collinear handles.
        let mut doc = doc_from("M 0 0 L 10 0 L 10 10", true);
        assert!(doc.paths[0].subpaths[0].nodes[1].handle_in.is_none());
        assert!(apply(
            &mut doc,
            &Op::SetNodeType {
                node: nref(0, 0, 1),
                node_type: NodeType::Smooth,
            }
        ));
        let n = &doc.paths[0].subpaths[0].nodes[1];
        assert_eq!(n.node_type, NodeType::Smooth);
        let hin = n.handle_in.expect("handle in");
        let hout = n.handle_out.expect("handle out");
        // out-handle and in-handle point opposite ways through the anchor (a smooth tangent).
        let cross =
            (hout.x - n.point.x) * (n.point.y - hin.y) - (hout.y - n.point.y) * (n.point.x - hin.x);
        assert!(cross.abs() < 1e-6, "handles should be collinear: {cross}");
    }

    #[test]
    fn combine_merges_subpaths_into_a_compound_path() {
        let mut doc = doc_from("M 0 0 L 10 0", false); // line
        apply(
            &mut doc,
            &Op::AddPath {
                id: "dome".into(),
                subpaths: parse_path_d("M 3 -2 Q 5 -6 7 -2"),
                attributes: IndexMap::new(),
            },
        );
        assert!(apply(
            &mut doc,
            &Op::CombinePaths {
                paths: vec![0, 1],
                id: "compound".into(),
            }
        ));
        assert!(doc.paths[0].deleted && doc.paths[1].deleted);
        let compound = doc.paths.last().unwrap();
        assert_eq!(compound.id, "compound");
        assert_eq!(compound.subpaths.len(), 2); // line + dome, kept distinct
        assert!(compound.added && !compound.deleted);
    }

    #[test]
    fn combine_inherits_fill_from_a_filled_member_not_a_stroke_only_one() {
        let mut doc = doc_from("M 0 0 L 10 0", false);
        let mut rim_attrs = IndexMap::new();
        rim_attrs.insert("fill".to_string(), "none".to_string()); // stroke-only (backmost)
        rim_attrs.insert("stroke".to_string(), "#000".to_string());
        apply(
            &mut doc,
            &Op::AddPath {
                id: "rim".into(),
                subpaths: parse_path_d("M 0 5 Q 5 8 10 5"),
                attributes: rim_attrs,
            },
        );
        let mut dome_attrs = IndexMap::new();
        dome_attrs.insert("fill".to_string(), "#808000".to_string()); // filled (front)
        apply(
            &mut doc,
            &Op::AddPath {
                id: "dome".into(),
                subpaths: parse_path_d("M 0 0 L 10 0 L 10 10 Z"),
                attributes: dome_attrs,
            },
        );
        // rim (index 1) is backmost but fill="none"; dome (index 2) fills → compound keeps the fill.
        assert!(apply(
            &mut doc,
            &Op::CombinePaths {
                paths: vec![1, 2],
                id: "compound".into(),
            }
        ));
        let compound = doc.paths.last().unwrap();
        assert_eq!(
            compound
                .attributes
                .as_ref()
                .and_then(|a| a.get("fill"))
                .map(|s| s.as_str()),
            Some("#808000"),
        );
    }

    #[test]
    fn release_splits_a_compound_into_independent_paths() {
        let mut doc = doc_from("M 0 0 L 10 0", false);
        apply(
            &mut doc,
            &Op::AddPath {
                id: "dome".into(),
                subpaths: parse_path_d("M 3 -2 Q 5 -6 7 -2"),
                attributes: IndexMap::new(),
            },
        );
        apply(
            &mut doc,
            &Op::CombinePaths {
                paths: vec![0, 1],
                id: "compound".into(),
            },
        );
        let compound_idx = doc.paths.len() - 1;
        assert!(apply(
            &mut doc,
            &Op::ReleaseCompound {
                path: compound_idx,
                ids: vec!["piece-a".into(), "piece-b".into()],
            }
        ));
        assert!(doc.paths[compound_idx].deleted); // source compound gone
        let live: Vec<&PathElement> = doc.paths.iter().filter(|p| !p.deleted).collect();
        assert_eq!(live.len(), 2); // one path per subpath
        assert!(live.iter().all(|p| p.subpaths.len() == 1 && p.added));
        let ids: Vec<&str> = live.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["piece-a", "piece-b"]);
        // releasing a single-subpath path is a no-op
        let last = doc.paths.len() - 1;
        assert!(!apply(
            &mut doc,
            &Op::ReleaseCompound {
                path: last,
                ids: vec!["x".into()],
            }
        ));
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
                spec: ShapeSpec::Rect {
                    x0: 10.0,
                    y0: 20.0,
                    x1: 30.0,
                    y1: 40.0,
                },
            },
        );
        assert_eq!(doc.paths[0].subpaths[0].nodes.len(), 4);
        assert!(doc.paths[0].subpaths[0].closed);

        apply(
            &mut doc,
            &Op::SetShape {
                path: 0,
                subpath: 0,
                spec: ShapeSpec::Line {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 5.0,
                    y1: 5.0,
                },
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
