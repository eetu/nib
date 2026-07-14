//! nib-core — the document / geometry / operation engine.
//!
//! This crate is the single source of truth for nib's editing logic. It compiles to:
//!   * **WASM** (`wasm-pack build core --target web`) — the browser editor drives it
//!     locally for 60fps, offline-capable editing.
//!   * **native** (`cargo build` / `cargo test`, and later the rust-axum backend) — the
//!     authority for persistence + realtime sync + the MCP tool surface.
//!
//! The [`Editor`] owns the document, the current selection, and the undo history. The JS
//! side holds one `Editor` handle and drives it with ops (mutations) and queries (render
//! state). Editing logic is deliberately split so it is testable natively: each JS-facing
//! `#[wasm_bindgen]` method is a thin serde wrapper over a plain-Rust core method.

use serde::Serialize;
use wasm_bindgen::prelude::*;

pub mod history;
pub mod model;
pub mod ops;
pub mod snap;

use history::History;
use model::document::{parse_svg, serialize_svg};
use model::types::{Gradient, Layer, NodeRef, PathElement, SvgDocument};
use ops::Op;

const BLANK_SVG: &str =
    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\">\n</svg>";

/// One undo step: the document's paths plus the selection that was active. Selection is
/// captured so undo/redo restore it, matching the TS store.
#[derive(Clone)]
struct Snapshot {
    paths: Vec<PathElement>,
    layers: Vec<Layer>,
    active_layer: Option<String>,
    gradients: Vec<Gradient>,
    selection: Option<NodeRef>,
    selected_path: Option<usize>,
}

/// The nib editing engine — owns the document, selection, and undo history.
#[wasm_bindgen]
pub struct Editor {
    doc: Option<SvgDocument>,
    selection: Option<NodeRef>,
    selected_path: Option<usize>,
    dirty: bool,
    history: History<Snapshot>,
}

// --- native core: used by the WASM surface below and directly by `cargo test` ---
impl Editor {
    fn snapshot(&self) -> Snapshot {
        let doc = self.doc.as_ref();
        Snapshot {
            paths: doc.map(|d| d.paths.clone()).unwrap_or_default(),
            layers: doc.map(|d| d.layers.clone()).unwrap_or_default(),
            active_layer: doc.and_then(|d| d.active_layer.clone()),
            gradients: doc.map(|d| d.gradients.clone()).unwrap_or_default(),
            selection: self.selection,
            selected_path: self.selected_path,
        }
    }

    fn restore(&mut self, snap: &Snapshot) {
        if let Some(doc) = self.doc.as_mut() {
            doc.paths = snap.paths.clone();
            doc.layers = snap.layers.clone();
            doc.active_layer = snap.active_layer.clone();
            doc.gradients = snap.gradients.clone();
        }
        self.selection = snap.selection;
        self.selected_path = snap.selected_path;
    }

    /// Replace the document from SVG source. Errors (without mutating) if it won't parse —
    /// so a failed edit in the SOURCE drawer leaves the current doc intact.
    pub fn load_source(&mut self, source: &str) -> Result<(), String> {
        let doc = parse_svg(source)?;
        self.doc = Some(doc);
        self.selection = None;
        self.selected_path = None;
        self.dirty = false;
        self.history.reset(self.snapshot());
        Ok(())
    }

    /// Apply one op as a live edit (no commit). Returns whether it mutated the document.
    pub fn apply(&mut self, op: &Op) -> bool {
        match self.doc.as_mut() {
            Some(doc) => ops::apply(doc, op),
            None => false,
        }
    }

    /// Apply a batch of ops as a live edit (no commit).
    pub fn apply_many(&mut self, ops: &[Op]) {
        if let Some(doc) = self.doc.as_mut() {
            for op in ops {
                ops::apply(doc, op);
            }
        }
    }

    /// Select a node (implies selecting its path); `None` clears the node selection.
    pub fn set_selection(&mut self, sel: Option<NodeRef>) {
        self.selection = sel;
        if let Some(r) = sel {
            self.selected_path = Some(r.path_index);
        }
    }

    /// Select a whole path with no node (clears the node selection).
    pub fn set_selected_path(&mut self, index: Option<usize>) {
        self.selected_path = index;
        self.selection = None;
    }

    pub fn doc(&self) -> Option<&SvgDocument> {
        self.doc.as_ref()
    }
    pub fn selection(&self) -> Option<NodeRef> {
        self.selection
    }
    pub fn selected_path(&self) -> Option<usize> {
        self.selected_path
    }
}

// --- WASM surface: thin serde wrappers over the core above ---
#[wasm_bindgen]
impl Editor {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Editor {
        Editor {
            doc: None,
            selection: None,
            selected_path: None,
            dirty: false,
            history: History::new(),
        }
    }

    /// Replace the document from SVG source; throws (leaving the doc untouched) on bad markup.
    #[wasm_bindgen(js_name = load)]
    pub fn load_js(&mut self, source: &str) -> Result<(), JsValue> {
        self.load_source(source).map_err(|e| JsValue::from_str(&e))
    }

    /// Create an empty document to draw on if none is loaded. No-op otherwise.
    #[wasm_bindgen(js_name = ensureBlank)]
    pub fn ensure_blank(&mut self) {
        if self.doc.is_none() {
            let _ = self.load_source(BLANK_SVG);
        }
    }

    pub fn clear(&mut self) {
        self.doc = None;
        self.selection = None;
        self.selected_path = None;
        self.dirty = false;
        self.history.reset(Snapshot {
            paths: Vec::new(),
            layers: Vec::new(),
            active_layer: None,
            gradients: Vec::new(),
            selection: None,
            selected_path: None,
        });
    }

    /// Replace the whole document from a serialized `SvgDocument` — used to rehydrate a
    /// persisted session, which stores the *edited* model, not just the source string.
    #[wasm_bindgen(js_name = setDocument)]
    pub fn set_document(&mut self, doc: JsValue) -> Result<(), JsValue> {
        let doc: SvgDocument = serde_wasm_bindgen::from_value(doc)?;
        self.doc = Some(doc);
        self.selection = None;
        self.selected_path = None;
        self.dirty = false;
        self.history.reset(self.snapshot());
        Ok(())
    }

    /// Apply one serialized `Op` as a live edit (no commit). Returns whether it mutated.
    #[wasm_bindgen(js_name = applyOp)]
    pub fn apply_op(&mut self, op: JsValue) -> Result<bool, JsValue> {
        let op: Op = serde_wasm_bindgen::from_value(op)?;
        Ok(self.apply(&op))
    }

    /// Apply a serialized array of `Op`s as a live edit (no commit).
    #[wasm_bindgen(js_name = applyOps)]
    pub fn apply_ops(&mut self, ops: JsValue) -> Result<(), JsValue> {
        let ops: Vec<Op> = serde_wasm_bindgen::from_value(ops)?;
        self.apply_many(&ops);
        Ok(())
    }

    /// Record the live-edited state as one undo step.
    pub fn commit(&mut self) {
        if self.doc.is_none() {
            return;
        }
        self.dirty = true;
        let snap = self.snapshot();
        self.history.commit(snap);
    }

    /// Abandon an in-flight gesture, restoring the last committed state.
    pub fn revert(&mut self) {
        if let Some(snap) = self.history.current().cloned() {
            self.restore(&snap);
        }
    }

    pub fn undo(&mut self) -> bool {
        if let Some(snap) = self.history.undo().cloned() {
            self.restore(&snap);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        if let Some(snap) = self.history.redo().cloned() {
            self.restore(&snap);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Select a node (implies its path). Pass `null` to clear the node selection.
    pub fn select(&mut self, node_ref: JsValue) -> Result<(), JsValue> {
        let sel: Option<NodeRef> = serde_wasm_bindgen::from_value(node_ref)?;
        self.set_selection(sel);
        Ok(())
    }

    #[wasm_bindgen(js_name = selectPath)]
    pub fn select_path(&mut self, index: Option<u32>) {
        self.set_selected_path(index.map(|i| i as usize));
    }

    pub fn deselect(&mut self) {
        self.selection = None;
        self.selected_path = None;
    }

    #[wasm_bindgen(js_name = markSaved)]
    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    /// Current document serialized back to SVG (unedited markup preserved byte-for-byte).
    #[wasm_bindgen(js_name = toSvg)]
    pub fn to_svg(&self) -> String {
        self.doc.as_ref().map(serialize_svg).unwrap_or_default()
    }

    #[wasm_bindgen(getter)]
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    #[wasm_bindgen(getter, js_name = canUndo)]
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    #[wasm_bindgen(getter, js_name = canRedo)]
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    #[wasm_bindgen(getter, js_name = hasDocument)]
    pub fn has_document(&self) -> bool {
        self.doc.is_some()
    }

    /// A snapshot of everything the UI needs to render, pulled after each mutation: the
    /// document, the selection, and the undo/dirty flags. Maps serialize as plain JS objects
    /// (not `Map`s) so `attributes.fill` reads work.
    pub fn state(&self) -> Result<JsValue, JsValue> {
        let state = EditorState {
            document: self.doc.as_ref(),
            selection: self.selection,
            selected_path: self.selected_path,
            dirty: self.dirty,
            can_undo: self.history.can_undo(),
            can_redo: self.history.can_redo(),
        };
        let serializer = serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);
        state.serialize(&serializer).map_err(Into::into)
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

/// The UI-facing editor snapshot (see [`Editor::state`]).
#[derive(Serialize)]
struct EditorState<'a> {
    document: Option<&'a SvgDocument>,
    selection: Option<NodeRef>,
    #[serde(rename = "selectedPath")]
    selected_path: Option<usize>,
    dirty: bool,
    #[serde(rename = "canUndo")]
    can_undo: bool,
    #[serde(rename = "canRedo")]
    can_redo: bool,
}

/// The core crate version, surfaced in the UI to confirm the WASM engine is loaded.
#[wasm_bindgen]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::types::Point;

    const SAMPLE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <path id="a" d="M 10 10 L 90 10 L 90 90" fill="none" stroke="black"/>
</svg>"##;

    fn nref(path: usize, subpath: usize, node: usize) -> NodeRef {
        NodeRef {
            path_index: path,
            subpath_index: subpath,
            node_index: node,
        }
    }

    #[test]
    fn version_matches_cargo() {
        assert_eq!(core_version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn load_then_export_round_trips_byte_for_byte() {
        let mut ed = Editor::new();
        ed.load_source(SAMPLE).unwrap();
        assert!(ed.has_document());
        assert!(!ed.can_undo());
        assert_eq!(ed.to_svg(), SAMPLE);
    }

    #[test]
    fn load_rejects_bad_markup_without_mutating() {
        let mut ed = Editor::new();
        ed.load_source(SAMPLE).unwrap();
        assert!(ed.load_source("<div>nope</div>").is_err());
        assert_eq!(ed.to_svg(), SAMPLE); // untouched
    }

    #[test]
    fn commit_then_undo_redo_restores_geometry() {
        let mut ed = Editor::new();
        ed.load_source(SAMPLE).unwrap();
        let before = ed.doc().unwrap().paths[0].subpaths[0].nodes[0].point;

        ed.apply(&Op::MoveNode {
            node: nref(0, 0, 0),
            to: Point::new(0.0, 0.0),
        });
        ed.commit();
        assert!(ed.can_undo());
        assert_eq!(
            ed.doc().unwrap().paths[0].subpaths[0].nodes[0].point,
            Point::new(0.0, 0.0)
        );

        assert!(ed.undo());
        assert_eq!(
            ed.doc().unwrap().paths[0].subpaths[0].nodes[0].point,
            before
        );
        assert!(ed.redo());
        assert_eq!(
            ed.doc().unwrap().paths[0].subpaths[0].nodes[0].point,
            Point::new(0.0, 0.0)
        );
    }

    #[test]
    fn revert_abandons_an_uncommitted_edit() {
        let mut ed = Editor::new();
        ed.load_source(SAMPLE).unwrap();
        let before = ed.doc().unwrap().paths[0].subpaths[0].nodes[0].point;
        ed.apply(&Op::MoveNode {
            node: nref(0, 0, 0),
            to: Point::new(1.0, 2.0),
        });
        ed.revert();
        assert_eq!(
            ed.doc().unwrap().paths[0].subpaths[0].nodes[0].point,
            before
        );
    }

    #[test]
    fn selection_helpers_mirror_the_ts_store() {
        let mut ed = Editor::new();
        ed.load_source(SAMPLE).unwrap();
        ed.set_selection(Some(nref(0, 0, 1)));
        assert_eq!(ed.selection(), Some(nref(0, 0, 1)));
        assert_eq!(ed.selected_path(), Some(0)); // node selection implies its path

        ed.set_selected_path(Some(3));
        assert_eq!(ed.selected_path(), Some(3));
        assert_eq!(ed.selection(), None); // path selection clears the node

        ed.deselect();
        assert_eq!(ed.selected_path(), None);
        assert_eq!(ed.selection(), None);
    }

    #[test]
    fn ops_round_trip_through_json() {
        // Validates the internally-tagged serde shape the WASM boundary (and MCP) rely on.
        let ops = vec![
            Op::MoveNode {
                node: nref(0, 0, 1),
                to: Point::new(3.5, -2.0),
            },
            Op::SetStyle {
                path: 0,
                key: "fill".into(),
                value: Some("red".into()),
            },
            Op::DeletePath { path: 2 },
        ];
        for op in ops {
            let json = serde_json::to_string(&op).unwrap();
            let back: Op = serde_json::from_str(&json).unwrap();
            assert_eq!(op, back, "op did not round-trip: {json}");
        }
        // spot-check the discriminant shape
        let json = serde_json::to_string(&Op::DeletePath { path: 2 }).unwrap();
        assert!(json.contains("\"type\":\"deletePath\""), "{json}");
    }
}
