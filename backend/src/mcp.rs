//! The MCP tool surface (Phase C3) — exposes nib's editing engine to an LLM over the Model
//! Context Protocol. It runs **in-process with the backend** (nested at `/mcp` via the
//! Streamable-HTTP transport) and shares one editing session, so the LLM and — later (C2) — the
//! live UI act on the same document. The tools are a thin layer over the same `nib-core` op
//! vocabulary the browser editor runs on: the ops ARE the surface.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use nib_core::Editor;
use nib_core::ops::Op;
use rmcp::handler::server::tool::{Parameters, ToolRouter};
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{ErrorData, ServerHandler, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;

use crate::safe_name;

/// The single editing session the MCP tools share (one document open at a time). Held behind an
/// `Arc<Mutex<…>>` so every MCP connection — and, later, the live UI — drives the same `Editor`.
pub struct Session {
    pub editor: Editor,
    /// The filename the current document was opened as (the default save target).
    pub name: Option<String>,
}

impl Session {
    pub fn new() -> Self {
        Session {
            editor: Editor::new(),
            name: None,
        }
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct NibMcp {
    session: Arc<Mutex<Session>>,
    docs: Arc<PathBuf>,
    tool_router: ToolRouter<NibMcp>,
}

impl NibMcp {
    pub fn new(session: Arc<Mutex<Session>>, docs: Arc<PathBuf>) -> Self {
        NibMcp {
            session,
            docs,
            tool_router: Self::tool_router(),
        }
    }
}

fn bad(msg: impl Into<String>) -> ErrorData {
    ErrorData::invalid_params(msg.into(), None)
}

/// A compact, structured view of the open document — what an LLM needs to plan edits: the
/// viewBox and each editable path (addressed by integer `index`) with its id, rough bounds,
/// fill/stroke, and node count. Not the raw markup (that's `get_svg`).
fn summary(s: &Session) -> serde_json::Value {
    let Some(doc) = s.editor.doc() else {
        return serde_json::json!({ "open": false });
    };
    let vb = &doc.view_box;
    let paths: Vec<serde_json::Value> = doc
        .paths
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.deleted)
        .map(|(i, p)| {
            let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for sp in &p.subpaths {
                for n in &sp.nodes {
                    minx = minx.min(n.point.x);
                    miny = miny.min(n.point.y);
                    maxx = maxx.max(n.point.x);
                    maxy = maxy.max(n.point.y);
                }
            }
            let style = |k: &str| {
                p.style_override
                    .as_ref()
                    .and_then(|s| s.get(k))
                    .or_else(|| p.attributes.as_ref().and_then(|a| a.get(k)))
                    .cloned()
            };
            let bounds = if minx <= maxx {
                serde_json::json!({ "x": minx, "y": miny, "w": maxx - minx, "h": maxy - miny })
            } else {
                serde_json::Value::Null
            };
            serde_json::json!({
                "index": i,
                "id": p.id,
                "added": p.added,
                "hidden": p.hidden,
                "nodes": p.subpaths.iter().map(|sp| sp.nodes.len()).sum::<usize>(),
                "bounds": bounds,
                "fill": style("fill"),
                "stroke": style("stroke"),
            })
        })
        .collect();
    serde_json::json!({
        "open": true,
        "name": s.name,
        "viewBox": { "minX": vb.min_x, "minY": vb.min_y, "width": vb.width, "height": vb.height },
        "paths": paths,
    })
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct OpenParams {
    /// The document filename to open — a bare `*.svg` in the server's docs folder.
    pub name: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ApplyOpParams {
    /// The operation as a JSON object tagged by `type` (the nib op vocabulary).
    pub op: serde_json::Value,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SaveParams {
    /// Target filename (a bare `*.svg`); defaults to the name the document was opened as.
    #[serde(default)]
    pub name: Option<String>,
}

#[tool_router]
impl NibMcp {
    #[tool(description = "List the .svg documents in the server's docs folder.")]
    fn list_documents(&self) -> Result<String, ErrorData> {
        let mut names: Vec<String> = std::fs::read_dir(self.docs.as_ref())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.to_ascii_lowercase().ends_with(".svg"))
            .collect();
        names.sort();
        Ok(serde_json::to_string(&names).unwrap_or_default())
    }

    #[tool(
        description = "Open a document from the docs folder into the editing session (replacing any current one). Returns a structured summary of its paths + viewBox."
    )]
    fn open_document(&self, Parameters(p): Parameters<OpenParams>) -> Result<String, ErrorData> {
        let name =
            safe_name(&p.name).ok_or_else(|| bad("invalid filename (must be a bare *.svg)"))?;
        let source = std::fs::read_to_string(self.docs.join(name))
            .map_err(|_| bad(format!("no such document: {name}")))?;
        let mut s = self.session.lock().unwrap();
        s.editor.load_source(&source).map_err(bad)?;
        s.name = Some(name.to_string());
        Ok(serde_json::to_string(&summary(&s)).unwrap_or_default())
    }

    #[tool(
        description = "Summarize the open document: its viewBox and every editable path (integer `index`, id, rough bounds, fill, stroke, node count). Address paths by `index` in ops."
    )]
    fn get_document(&self) -> Result<String, ErrorData> {
        let s = self.session.lock().unwrap();
        Ok(serde_json::to_string(&summary(&s)).unwrap_or_default())
    }

    #[tool(description = "Get the open document serialized to SVG markup.")]
    fn get_svg(&self) -> Result<String, ErrorData> {
        let s = self.session.lock().unwrap();
        if !s.editor.has_document() {
            return Err(bad("no document open"));
        }
        Ok(s.editor.to_svg())
    }

    #[tool(
        description = "Apply one editing operation to the open document — the full nib op vocabulary. `op` is a JSON object tagged by `type`. Examples: {\"type\":\"movePathBy\",\"path\":0,\"dx\":10,\"dy\":0}; {\"type\":\"setStyle\",\"path\":0,\"key\":\"fill\",\"value\":\"#ff0000\"}; {\"type\":\"addShape\",\"id\":\"c1\",\"spec\":{\"shape\":\"ellipse\",\"cx\":50,\"cy\":50,\"rx\":20,\"ry\":20},\"attributes\":{\"fill\":\"#0088ff\"}}; {\"type\":\"addPath\",\"id\":\"p1\",\"subpaths\":[...],\"attributes\":{...}}; {\"type\":\"booleanOp\",\"op\":\"union\",\"paths\":[0,1],\"id\":\"u1\"}; {\"type\":\"deletePath\",\"path\":2}; {\"type\":\"groupNodes\",\"uids\":[...],\"uid\":\"g1\",\"name\":\"group\"}. Returns the updated document summary, or an error if the op was invalid or matched no target."
    )]
    fn apply_op(&self, Parameters(p): Parameters<ApplyOpParams>) -> Result<String, ErrorData> {
        let op: Op = serde_json::from_value(p.op).map_err(|e| bad(format!("invalid op: {e}")))?;
        let mut s = self.session.lock().unwrap();
        if !s.editor.has_document() {
            return Err(bad("no document open — open_document first"));
        }
        if !s.editor.apply(&op) {
            return Err(bad("the op did not apply (missing target / no-op)"));
        }
        s.editor.commit(); // one op = one undo step, like the UI
        Ok(serde_json::to_string(&summary(&s)).unwrap_or_default())
    }

    #[tool(
        description = "Save the open document to the docs folder (validated through the parser). Defaults to the name it was opened as; pass `name` to save under a different file."
    )]
    fn save_document(&self, Parameters(p): Parameters<SaveParams>) -> Result<String, ErrorData> {
        let mut s = self.session.lock().unwrap();
        if !s.editor.has_document() {
            return Err(bad("no document open"));
        }
        let target = p
            .name
            .or_else(|| s.name.clone())
            .ok_or_else(|| bad("no filename — open a document first or pass `name`"))?;
        let name = safe_name(&target)
            .ok_or_else(|| bad("invalid filename (must be a bare *.svg)"))?
            .to_string();
        let svg = s.editor.to_svg();
        // Never persist markup the editor can't reopen (matches the /api file writes).
        nib_core::model::document::parse_svg(&svg).map_err(|e| {
            ErrorData::internal_error(format!("refusing to save invalid svg: {e}"), None)
        })?;
        std::fs::create_dir_all(self.docs.as_ref()).ok();
        std::fs::write(self.docs.join(&name), &svg)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        s.name = Some(name.clone());
        Ok(format!("saved {name} ({} bytes)", svg.len()))
    }
}

#[tool_handler]
impl ServerHandler for NibMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "nib is an SVG path editor. Workflow: list_documents → open_document → \
                 get_document (paths are addressed by integer `index`) → apply_op (one edit each) \
                 → save_document. Coordinates are in the document's viewBox units."
                    .to_string(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mcp_at(dir: &std::path::Path) -> NibMcp {
        NibMcp::new(
            Arc::new(Mutex::new(Session::new())),
            Arc::new(dir.to_path_buf()),
        )
    }

    #[test]
    fn open_edit_save_roundtrip() {
        let dir = std::env::temp_dir().join("nib-mcp-test-roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect id="r" x="10" y="10" width="30" height="30" fill="#000000"/></svg>"##;
        std::fs::write(dir.join("t.svg"), src).unwrap();

        let mcp = mcp_at(&dir);

        // list finds the file
        assert!(mcp.list_documents().unwrap().contains("t.svg"));

        // open → a summary with one editable path at index 0
        let sum = mcp
            .open_document(Parameters(OpenParams {
                name: "t.svg".into(),
            }))
            .unwrap();
        assert!(sum.contains("\"open\":true"), "summary: {sum}");
        assert!(sum.contains("\"index\":0"), "path indexed: {sum}");

        // apply_op: set the fill via the full op vocabulary
        let op =
            serde_json::json!({ "type": "setStyle", "path": 0, "key": "fill", "value": "#ff0000" });
        let after = mcp.apply_op(Parameters(ApplyOpParams { op })).unwrap();
        assert!(
            after.contains("#ff0000"),
            "summary reflects the edit: {after}"
        );

        // the edit is reflected in the serialized markup
        assert!(
            mcp.get_svg().unwrap().contains("#ff0000"),
            "svg reflects the edit"
        );

        // save under a new name → a valid, reopenable file with the edit
        let saved = mcp
            .save_document(Parameters(SaveParams {
                name: Some("out.svg".into()),
            }))
            .unwrap();
        assert!(saved.contains("out.svg"), "saved: {saved}");
        let written = std::fs::read_to_string(dir.join("out.svg")).unwrap();
        assert!(written.contains("#ff0000"), "persisted markup has the edit");
        assert!(
            nib_core::model::document::parse_svg(&written).is_ok(),
            "persisted svg reopens"
        );

        // a malformed op is rejected without panicking
        assert!(
            mcp.apply_op(Parameters(ApplyOpParams {
                op: serde_json::json!({ "type": "notAnOp" }),
            }))
            .is_err()
        );

        // tools on a fresh (no-document) session error gracefully
        let fresh = mcp_at(&dir);
        assert!(fresh.get_svg().is_err());
        assert!(
            fresh
                .apply_op(Parameters(ApplyOpParams {
                    op: serde_json::json!({ "type": "deletePath", "path": 0 }),
                }))
                .is_err()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
