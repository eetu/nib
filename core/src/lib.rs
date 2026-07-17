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

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

pub mod history;
pub mod model;
pub mod ops;
pub mod snap;

use history::History;
use model::document::{parse_svg, serialize_canonical, serialize_svg, tree_boolean_results};
use model::tree::{Tree, parse_tree};
use model::types::{Gradient, NodeRef, PathElement, Subpath, SvgDocument};
use ops::Op;

const BLANK_SVG: &str =
    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\">\n</svg>";

/// The native document-model format version — bump when the persisted JSON shape changes so a
/// loader can migrate older blobs.
const SCHEMA_VERSION: u32 = 1;

/// The native document model — nib's **single source of truth**, persisted + transmitted as JSON.
/// It carries the `SvgDocument` (paths, gradients, source) **plus** its structural `tree` (which is
/// `serde(skip)` on the doc, so it rides here explicitly). Every node's `uid` lives in this blob, so
/// a client that loads a model shares identity with whoever created it — it never re-parses SVG to
/// sync, which is what made uids drift. SVG (and any future format) is import/export only.
#[derive(Serialize, Deserialize)]
struct Model {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    document: SvgDocument,
    tree: Option<Tree>,
}

/// One undo step: the document's paths plus the selection that was active. Selection is
/// captured so undo/redo restore it, matching the TS store.
#[derive(Clone)]
struct Snapshot {
    paths: Vec<PathElement>,
    gradients: Vec<Gradient>,
    /// The structural tree — captured so undo/redo restore structural edits (group/reorder/…),
    /// not just geometry.
    tree: Option<Tree>,
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
            gradients: doc.map(|d| d.gradients.clone()).unwrap_or_default(),
            tree: doc.and_then(|d| d.tree.clone()),
            selection: self.selection,
            selected_path: self.selected_path,
        }
    }

    fn restore(&mut self, snap: &Snapshot) {
        if let Some(doc) = self.doc.as_mut() {
            doc.paths = snap.paths.clone();
            doc.gradients = snap.gradients.clone();
            doc.tree = snap.tree.clone();
        }
        self.selection = snap.selection;
        self.selected_path = snap.selected_path;
    }

    /// Replace the document from SVG source. Errors (without mutating) if it won't parse —
    /// so a failed edit in the SOURCE drawer leaves the current doc intact.
    pub fn load_source(&mut self, source: &str) -> Result<(), String> {
        let mut doc = parse_svg(source)?;
        // The model's paths come from the tree projection — so imported primitives (rect/circle/
        // …) are editable paths, and each carries the `uid` linking it back to its tree node.
        if let Some(tree) = &doc.tree {
            doc.paths = tree.project_paths();
        }
        self.doc = Some(doc);
        self.selection = None;
        self.selected_path = None;
        self.dirty = false;
        self.history.reset(self.snapshot());
        Ok(())
    }

    /// Build the native model (doc + structural tree) from the current document, `None` if empty.
    fn build_model(&self) -> Option<Model> {
        let doc = self.doc.as_ref()?;
        Some(Model {
            schema_version: SCHEMA_VERSION,
            document: doc.clone(),
            tree: doc.tree.clone(),
        })
    }

    /// Replace the whole document from a native model (doc + tree), ids intact — no SVG re-parse.
    fn set_model(&mut self, model: Model) {
        let mut doc = model.document;
        doc.tree = model.tree; // the tree is serde(skip) on the doc, so restore it from the model
        self.doc = Some(doc);
        self.selection = None;
        self.selected_path = None;
        self.dirty = false;
        self.history.reset(self.snapshot());
    }

    /// Serialize the document to the native model JSON — what the backend persists + serves. `None`
    /// if there's no document.
    pub fn to_model_json(&self) -> Option<String> {
        self.build_model()
            .and_then(|m| serde_json::to_string(&m).ok())
    }

    /// Load the document from native model JSON (ids intact). Errors (without mutating) on bad JSON.
    pub fn load_model_json(&mut self, json: &str) -> Result<(), String> {
        let model: Model = serde_json::from_str(json).map_err(|e| e.to_string())?;
        self.set_model(model);
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
            gradients: Vec::new(),
            tree: None,
            selection: None,
            selected_path: None,
        });
    }

    /// Replace the whole document from a serialized `SvgDocument` — used to rehydrate a
    /// persisted session, which stores the *edited* model, not just the source string.
    #[wasm_bindgen(js_name = setDocument)]
    pub fn set_document(&mut self, doc: JsValue) -> Result<(), JsValue> {
        let mut doc: SvgDocument = serde_wasm_bindgen::from_value(doc)?;
        // A persisted session carries its structural tree; an older one (or a doc set without a
        // tree) rebuilds it from the source — deterministic uids line the projection back up.
        if doc.tree.is_none() {
            doc.tree = parse_tree(&doc.source).ok();
        }
        self.doc = Some(doc);
        self.selection = None;
        self.selected_path = None;
        self.dirty = false;
        self.history.reset(self.snapshot());
        Ok(())
    }

    /// Serialize the document to the native model (doc + tree) for the frontend to persist/send —
    /// the single source of truth. `null` if there's no document. Maps serialize as plain objects.
    #[wasm_bindgen(js_name = toModel)]
    pub fn to_model_js(&self) -> Result<JsValue, JsValue> {
        match self.build_model() {
            Some(model) => {
                let serializer =
                    serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);
                model.serialize(&serializer).map_err(Into::into)
            }
            None => Ok(JsValue::NULL),
        }
    }

    /// Load the document from a native model (doc + tree) — ids intact, no SVG re-parse. The
    /// connected-mode open path (a client hydrates from the backend's model, sharing identity).
    #[wasm_bindgen(js_name = loadModel)]
    pub fn load_model_js(&mut self, json: JsValue) -> Result<(), JsValue> {
        let model: Model = serde_wasm_bindgen::from_value(json)?;
        self.set_model(model);
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

    /// Serialize the structural tree for persistence — it's `serde(skip)` on the doc (kept off
    /// the per-frame `state()` payload), so it's saved separately + only when the debounced
    /// persist runs. `null` if there's no tree.
    #[wasm_bindgen(js_name = treeJson)]
    pub fn tree_json(&self) -> Result<JsValue, JsValue> {
        let tree = self.doc.as_ref().and_then(|d| d.tree.clone());
        serde_wasm_bindgen::to_value(&tree).map_err(Into::into)
    }

    /// Restore a persisted structural tree (overriding the source-rebuilt one), so structural
    /// edits (group/hide/reorder) survive a session reload. Resets the undo baseline to match.
    #[wasm_bindgen(js_name = setTree)]
    pub fn set_tree(&mut self, json: JsValue) -> Result<(), JsValue> {
        let tree: Option<Tree> = serde_wasm_bindgen::from_value(json)?;
        if let Some(doc) = self.doc.as_mut() {
            if tree.is_some() {
                doc.tree = tree;
            }
        }
        self.history.reset(self.snapshot());
        Ok(())
    }

    /// Reconcile drawn (`added`) paths into the tree — a no-op for a current session, but migrates
    /// one persisted before drawn content lived in the tree (its added paths had no tree node) by
    /// appending a `<path>` node per orphan. Called once after `setDocument`/`setTree` on load.
    #[wasm_bindgen(js_name = syncDrawn)]
    pub fn sync_drawn(&mut self) {
        if let Some(doc) = self.doc.as_mut() {
            ops::ensure_drawn_in_tree(doc);
        }
        self.history.reset(self.snapshot());
    }

    /// The document's render tree (the root `<svg>`'s children) — what the canvas draws
    /// declaratively. The frontend fetches this per source change; edits pull live geometry from
    /// `doc.paths` by uid, structural ops re-fetch it.
    #[wasm_bindgen(js_name = renderTree)]
    pub fn render_tree(&self) -> Result<JsValue, JsValue> {
        let nodes = self
            .doc
            .as_ref()
            .and_then(|d| d.tree.as_ref())
            .map(|t| t.render_children())
            .unwrap_or_default();
        let serializer = serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);
        nodes.serialize(&serializer).map_err(Into::into)
    }

    /// Current document exported to SVG — the **canonical** export (the native model is the source
    /// of truth now): every element regenerated cleanly from the model, primitives kept as
    /// primitives (only freeform-reshaped shapes become `<path>`). For an exact byte-preserving
    /// export use the model's faithful path; for paths-only use `toSvgNormalized`.
    #[wasm_bindgen(js_name = toSvg)]
    pub fn to_svg(&self) -> String {
        match &self.doc {
            Some(doc) => match &doc.tree {
                Some(tree) => serialize_canonical(doc, tree, 3),
                // Fallback (no tree, e.g. a doc set without source): the flat splice serializer.
                None => serialize_svg(doc),
            },
            None => String::new(),
        }
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
        // Compute live-boolean group results here (not stored on the doc) so the UI can render
        // the baked geometry while the operands stay editable in `document.paths`. Derived from
        // the tree's `<g boolean_op>` nodes over the live `doc.paths` geometry, keyed by group uid.
        let boolean_results: Vec<BooleanResultDto> = self
            .doc
            .as_ref()
            .map(|doc| {
                tree_boolean_results(doc)
                    .into_iter()
                    .map(|r| BooleanResultDto {
                        uid: r.uid,
                        subpaths: r.subpaths,
                        attributes: r.attributes,
                        operand_uids: r.operand_uids,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let state = EditorState {
            document: self.doc.as_ref(),
            selection: self.selection,
            selected_path: self.selected_path,
            dirty: self.dirty,
            can_undo: self.history.can_undo(),
            can_redo: self.history.can_redo(),
            boolean_results,
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
    /// Computed geometry of each live-boolean group (derived, not stored on the doc).
    #[serde(rename = "booleanResults")]
    boolean_results: Vec<BooleanResultDto>,
}

/// A live-boolean group's computed render geometry + the paint it inherits (subject style).
#[derive(Serialize)]
struct BooleanResultDto {
    /// The boolean group node's stable uid (the canvas keys the painted result on it).
    uid: String,
    subpaths: Vec<Subpath>,
    attributes: IndexMap<String, String>,
    /// Uids of the group's operand paths — so the overlay can outline the editable sources.
    #[serde(rename = "operandUids")]
    operand_uids: Vec<String>,
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
    fn canonical_export_is_valid_and_a_fixed_point() {
        // The default export is canonical (regenerated from the model), not byte-preserving — so
        // it need not equal the source, but it must be valid + stable (parse→export→export is a
        // fixed point) and keep the path's geometry.
        let mut ed = Editor::new();
        ed.load_source(SAMPLE).unwrap();
        assert!(ed.has_document());
        assert!(!ed.can_undo());
        let out = ed.to_svg();
        assert!(out.contains("M 10 10"), "geometry preserved: {out}");
        // The root namespace MUST survive regeneration — without it the file isn't a valid
        // standalone SVG (browsers/Illustrator/Inkscape won't open it). roxmltree hides `xmlns` as a
        // namespace (not an attr), so the canonical (regenerated) open tag would drop it if we didn't
        // capture declared namespaces at parse.
        assert!(
            out.contains(r#"xmlns="http://www.w3.org/2000/svg""#),
            "canonical export keeps the SVG namespace: {out}"
        );
        // Re-loading the export and re-exporting yields the same bytes (no drift each save).
        let mut ed2 = Editor::new();
        ed2.load_source(&out).unwrap();
        assert_eq!(ed2.to_svg(), out, "canonical export is a fixed point");
    }

    #[test]
    fn canonical_export_keeps_namespaces_without_leaking_them_to_children() {
        // A doc that declares both the SVG default namespace AND xmlns:xlink on the root: canonical
        // export must re-emit both on <svg> exactly once, and NOT sprinkle xmlns onto child elements
        // (the default namespace is inherited, so children must not re-declare it).
        let mut ed = Editor::new();
        ed.load_source(
            r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 100 100"><rect x="1" y="1" width="8" height="8"/></svg>"##,
        )
        .unwrap();
        // Force a regenerate (canonical) by editing the rect's fill.
        ed.apply(&crate::ops::Op::SetStyle {
            path: 0,
            key: "fill".into(),
            value: Some("#f00".into()),
        });
        let out = ed.to_svg();
        assert_eq!(
            out.matches(r#"xmlns="http://www.w3.org/2000/svg""#).count(),
            1,
            "default namespace declared exactly once, on the root: {out}"
        );
        assert!(
            out.contains(r#"xmlns:xlink="http://www.w3.org/1999/xlink""#),
            "xlink namespace preserved: {out}"
        );
    }

    #[test]
    fn canonical_export_gives_use_old_illustrator_xlink_backcompat() {
        // A component + a <use> written with SVG2 bare `href`. Canonical export must ADD `xlink:href`
        // (old Illustrator only resolves that) + declare `xmlns:xlink`, keep the `href`, and stay a
        // fixed point (no duplicate attrs/decls on the round-trip).
        let mut ed = Editor::new();
        ed.load_source(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><g id="die"><rect x="0" y="0" width="9" height="9"/></g></defs><use href="#die" x="10" y="10"/></svg>"##,
        )
        .unwrap();
        let out = ed.to_svg();
        assert!(
            out.contains(r#"xmlns:xlink="http://www.w3.org/1999/xlink""#),
            "root declares xlink: {out}"
        );
        assert!(
            out.contains(r##"xlink:href="#die""##),
            "use mirrors href → xlink:href: {out}"
        );
        assert!(
            out.contains(r##"href="#die""##),
            "keeps the SVG2 href too: {out}"
        );
        assert_eq!(
            out.matches("xlink:href").count(),
            1,
            "no dup xlink:href: {out}"
        );
        assert_eq!(
            out.matches("xmlns:xlink").count(),
            1,
            "one xmlns:xlink: {out}"
        );
        let mut ed2 = Editor::new();
        ed2.load_source(&out).unwrap();
        assert_eq!(
            ed2.to_svg(),
            out,
            "canonical export with xlink compat is a fixed point"
        );
    }

    #[test]
    fn canonical_export_preserves_namespaced_attribute_prefixes() {
        // roxmltree drops attribute prefixes (`xlink:href` → `href`); canonical regeneration must
        // reconstruct them, or an `<image xlink:href>` (or inkscape:*/sodipodi:*) would be mangled.
        let mut ed = Editor::new();
        ed.load_source(
            r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 100 100"><image xlink:href="pic.png" x="0" y="0" width="10" height="10"/><rect x="1" y="1" width="2" height="2"/></svg>"##,
        )
        .unwrap();
        let out = ed.to_svg();
        assert!(
            out.contains(r#"xlink:href="pic.png""#),
            "namespaced attr prefix preserved (not mangled to bare href): {out}"
        );
    }

    #[test]
    fn model_json_round_trip_preserves_uids_and_svg() {
        // The native model is the source of truth: loading it must reproduce identical node uids
        // (so structural ops replay across clients) and the same document — no SVG re-parse.
        let mut ed = Editor::new();
        ed.load_source(SAMPLE).unwrap();
        let uids0: Vec<String> = ed
            .doc()
            .unwrap()
            .paths
            .iter()
            .map(|p| p.uid.clone())
            .collect();
        let svg0 = ed.to_svg();
        let model = ed.to_model_json().expect("model json");

        let mut ed2 = Editor::new();
        ed2.load_model_json(&model).unwrap();
        let uids1: Vec<String> = ed2
            .doc()
            .unwrap()
            .paths
            .iter()
            .map(|p| p.uid.clone())
            .collect();
        assert!(
            !uids0.is_empty() && !uids0[0].is_empty(),
            "paths carry uids"
        );
        assert_eq!(uids0, uids1, "uids survive the model round-trip");
        assert!(ed2.doc().unwrap().tree.is_some(), "tree rides in the model");
        assert_eq!(ed2.to_svg(), svg0, "svg round-trips through the model");
    }

    #[test]
    fn load_rejects_bad_markup_without_mutating() {
        let mut ed = Editor::new();
        ed.load_source(SAMPLE).unwrap();
        let good = ed.to_svg();
        assert!(ed.load_source("<div>nope</div>").is_err());
        assert_eq!(ed.to_svg(), good); // untouched by the failed load
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
