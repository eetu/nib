//! In-memory project sessions (Phase C). Each open project has ONE authoritative `nib_core::Editor`
//! keyed by project id, shared by every client editing it (the MCP connection + the browser over
//! WebSocket). All edits funnel through [`apply_ops`]: mutate the editor, broadcast the ops to the
//! project's other subscribers, and persist the native model (+ a cached SVG export) to SQLite.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use nib_core::Editor;
use nib_core::ops::Op;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::broadcast;

use crate::db;

pub type Sessions = Arc<Mutex<HashMap<i64, Arc<Mutex<ProjectSession>>>>>;

pub fn new_sessions() -> Sessions {
    Arc::new(Mutex::new(HashMap::new()))
}

/// A batch of ops broadcast to a project's subscribers. `client_id` is the origin, so a client
/// ignores the echo of its own edits. Ops replay cleanly on every client now that all clients load
/// the **same native model** (node `uid`s are shared identity carried by the model), so even
/// structural uid-ops (`groupNodes`, `reorderNode`, …) replay correctly — no snapshot resync needed.
#[derive(Clone, Serialize, Deserialize)]
pub struct SyncMsg {
    #[serde(rename = "clientId")]
    pub client_id: String,
    pub ops: Vec<serde_json::Value>,
}

/// Stamp a fresh globally-unique `uid` (or `uids`, for `releaseCompound`) onto a create-op that
/// lacks one — so the node's identity is minted once and carried, never re-invented by a peer
/// replaying the op. A no-op for non-create ops and for ops that already carry a uid.
fn ensure_create_uid(op: &mut serde_json::Value) {
    let Some(ty) = op.get("type").and_then(|t| t.as_str()) else {
        return;
    };
    match ty {
        "releaseCompound" => {
            let n = op
                .get("ids")
                .and_then(|v| v.as_array())
                .map_or(0, |a| a.len());
            let has = op
                .get("uids")
                .and_then(|v| v.as_array())
                .is_some_and(|a| !a.is_empty());
            if n > 0 && !has {
                let uids: Vec<String> = (0..n).map(|_| nib_core::model::tree::new_id()).collect();
                op["uids"] = serde_json::json!(uids);
            }
        }
        "createComponent" => {
            for key in ["uid", "useUid", "defsUid"] {
                if op.get(key).and_then(|v| v.as_str()).is_none() {
                    op[key] = serde_json::json!(nib_core::model::tree::new_id());
                }
            }
        }
        "detachInstance" => {
            // The baked wrapper <g>'s uid; its descendants derive deterministically from it.
            if op.get("gUid").and_then(|v| v.as_str()).is_none() {
                op["gUid"] = serde_json::json!(nib_core::model::tree::new_id());
            }
        }
        "addPath" | "addShape" | "booleanOp" | "combinePaths" | "outlineStroke" | "offsetPath"
        | "stampInstance" | "setDropShadow" | "addText" => {
            if op.get("uid").and_then(|v| v.as_str()).is_none() {
                op["uid"] = serde_json::json!(nib_core::model::tree::new_id());
            }
        }
        _ => {}
    }
}

/// The authoritative in-memory session for one open project.
pub struct ProjectSession {
    pub project_id: i64,
    pub editor: Editor,
    pub tx: broadcast::Sender<SyncMsg>,
}

/// Get (or lazily load from the DB) the session for a project the `user_id` owns.
pub async fn open(
    pool: &SqlitePool,
    sessions: &Sessions,
    user_id: i64,
    project_id: i64,
) -> Result<Arc<Mutex<ProjectSession>>, String> {
    if let Some(s) = sessions.lock().unwrap().get(&project_id).cloned() {
        return Ok(s);
    }
    let project = db::get_project(pool, user_id, project_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no such project: {project_id}"))?;
    let mut editor = Editor::new();
    if project.model.is_empty() {
        // Legacy / freshly-created row: import the SVG once, then persist the native model so every
        // later open (and every client) shares node identity instead of re-parsing.
        editor.load_source(&project.svg)?;
        if let Some(model) = editor.to_model_json() {
            let _ = db::update_project(pool, project_id, &model, &editor.to_svg()).await;
        }
    } else {
        editor.load_model_json(&project.model)?;
    }
    let (tx, _rx) = broadcast::channel(256);
    let session = Arc::new(Mutex::new(ProjectSession {
        project_id,
        editor,
        tx,
    }));
    sessions.lock().unwrap().insert(project_id, session.clone());
    Ok(session)
}

/// Apply a batch of JSON ops to a project session: mutate the editor (one undo step), broadcast the
/// ops to the other subscribers, and persist the native model (+ cached SVG) to SQLite. Returns how
/// many ops applied. Sync (so the MCP tools can call it directly); the DB write is spawned.
pub fn apply_ops(
    session: &Arc<Mutex<ProjectSession>>,
    pool: &SqlitePool,
    mut ops: Vec<serde_json::Value>,
    origin: &str,
) -> Result<usize, String> {
    // A create-op mints a new node's identity; stamp a uid if the caller didn't (the LLM's raw
    // apply_op, an MCP wrapper), so the applied + broadcast op carries it and every client agrees.
    for op in ops.iter_mut() {
        ensure_create_uid(op);
    }
    let parsed: Vec<Op> = ops
        .iter()
        .map(|v| serde_json::from_value(v.clone()).map_err(|e| format!("invalid op: {e}")))
        .collect::<Result<_, _>>()?;
    let (model, svg, id, applied) = {
        let mut s = session.lock().unwrap();
        let mut applied = 0usize;
        for op in &parsed {
            if s.editor.apply(op) {
                applied += 1;
            }
        }
        if applied == 0 {
            return Ok(0);
        }
        s.editor.commit();
        // The native model is the source of truth (persisted); the svg is a cached export.
        let model = s.editor.to_model_json().unwrap_or_default();
        let svg = s.editor.to_svg();
        // Broadcast the ops to the project's other subscribers. All clients load the same model
        // (identical node uids), so every op — structural uid-ops included — replays correctly.
        let _ = s.tx.send(SyncMsg {
            client_id: origin.to_string(),
            ops,
        });
        (model, svg, s.project_id, applied)
    };
    // Persist model + cached svg (fire-and-forget; the editor is the live authority meanwhile).
    let pool = pool.clone();
    tokio::spawn(async move {
        let _ = db::update_project(&pool, id, &model, &svg).await;
    });
    Ok(applied)
}
