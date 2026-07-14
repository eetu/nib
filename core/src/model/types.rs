//! Core data types — ported from `model/types.ts`. Serde field renames keep the JSON shape
//! identical to the TS model so the same values cross the WASM boundary unchanged.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// A 2D point in document coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }
}

/// A smooth node keeps its two handles collinear (mirror on drag); a corner node moves
/// them independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    Corner,
    Smooth,
}

/// An anchor point plus its optional bezier control handles, in absolute document
/// coordinates. A segment between two adjacent nodes is a straight line iff the outgoing
/// handle of the first and the incoming handle of the second are both absent.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PathNode {
    pub point: Point,
    #[serde(rename = "handleIn", skip_serializing_if = "Option::is_none", default)]
    pub handle_in: Option<Point>,
    #[serde(rename = "handleOut", skip_serializing_if = "Option::is_none", default)]
    pub handle_out: Option<Point>,
    #[serde(rename = "type")]
    pub node_type: NodeType,
}

impl PathNode {
    /// A corner node with no handles (a straight-segment anchor).
    pub fn corner(point: Point) -> Self {
        PathNode {
            point,
            handle_in: None,
            handle_out: None,
            node_type: NodeType::Corner,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subpath {
    pub nodes: Vec<PathNode>,
    pub closed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ViewBox {
    #[serde(rename = "minX")]
    pub min_x: f64,
    #[serde(rename = "minY")]
    pub min_y: f64,
    pub width: f64,
    pub height: f64,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn default_true() -> bool {
    true
}

/// A named layer — a flat, ordered organizational grouping over paths. New shapes land on the
/// active layer; layers give z-order + show/hide, and export as top-level `<g>` wrappers. An
/// LLM organizes generated shapes onto layers far more cleanly than into a flat path list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub visible: bool,
}

/// A single `<path>` element: its editable model plus what we need to write the edit back
/// into the original SVG source without disturbing anything else.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathElement {
    /// Stable id for selection — the element's `id` attr, else `path-<index>`.
    pub id: String,
    /// 0-based position among `<path>` elements in document order.
    pub index: usize,
    /// The `d` attribute exactly as it appeared in the source.
    #[serde(rename = "originalD")]
    pub original_d: String,
    pub subpaths: Vec<Subpath>,
    /// Set once the user changes the geometry — only edited paths get their `d`
    /// re-serialized on export; everything else is preserved verbatim.
    pub edited: bool,
    /// True for paths drawn in-app: not present in the source, so they're appended on
    /// export and rendered from the model.
    #[serde(skip_serializing_if = "is_false", default)]
    pub added: bool,
    /// Presentation attributes. For added paths this is the whole style; for imported paths
    /// it's the style parsed from source (display + reset).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub attributes: Option<IndexMap<String, String>>,
    /// Style edits to an imported path — merged over `attributes` and spliced into the
    /// source tag on export.
    #[serde(
        rename = "styleOverride",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub style_override: Option<IndexMap<String, String>>,
    /// The imported path's opening `<path …>` tag exactly as in the source — the anchor for
    /// surgical d/style rewrites on export.
    #[serde(
        rename = "originalTag",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub original_tag: Option<String>,
    /// Soft-deleted: kept so indices stay stable + undo restores it, but omitted from
    /// render, hit-testing, and export.
    #[serde(skip_serializing_if = "is_false", default)]
    pub deleted: bool,
    /// The user renamed this path — write its `id` into the exported markup.
    #[serde(skip_serializing_if = "is_false", default)]
    pub renamed: bool,
    /// Id of the layer this path belongs to (`None` = unassigned / the implicit default).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub layer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SvgDocument {
    /// Original SVG text, kept so unedited markup exports byte-for-byte.
    pub source: String,
    #[serde(rename = "viewBox")]
    pub view_box: ViewBox,
    pub paths: Vec<PathElement>,
    /// Named layers, in z-order (bottom → top). Empty = no explicit layers → the document
    /// exports via the byte-preserving splice; once populated, drawn paths group into `<g>`s.
    #[serde(default)]
    pub layers: Vec<Layer>,
    /// The layer new shapes are added to (`None` = unassigned).
    #[serde(rename = "activeLayer", skip_serializing_if = "Option::is_none", default)]
    pub active_layer: Option<String>,
}

/// Addresses one anchor node inside the document — the unit of selection and the identity a
/// drag operates on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRef {
    #[serde(rename = "pathIndex")]
    pub path_index: usize,
    #[serde(rename = "subpathIndex")]
    pub subpath_index: usize,
    #[serde(rename = "nodeIndex")]
    pub node_index: usize,
}

/// Nullable-aware node-ref equality (mirrors `nodeRefEquals`): two `None`s are equal, a
/// `None` and a `Some` are not.
pub fn node_ref_equals(a: Option<NodeRef>, b: Option<NodeRef>) -> bool {
    a == b
}
