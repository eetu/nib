//! In-memory project sessions (Phase C). Each open project has ONE authoritative `nib_core::Editor`
//! keyed by project id, shared by every client editing it (the MCP connection now; the browser over
//! WebSocket in C2). All edits funnel through [`apply_ops`]: mutate the editor, broadcast the ops to
//! the project's other subscribers, and persist the new SVG to SQLite.

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
/// ignores the echo of its own edits.
///
/// Most ops replay cleanly on every client (they address paths by *index*, which all clients agree
/// on). But **tree-structural ops address nodes by `uid`** — and uids are per-client in-memory
/// identities (the backend builds its tree incrementally via `fresh_uid`; a browser builds its by
/// parsing the SVG), so they don't match across clients. Replaying a `groupNodes` with the origin's
/// uids on a peer groups the wrong nodes. So for those ops we instead ship the authoritative post-op
/// `svg` snapshot: the peer reloads it and its tree matches structurally, uids irrelevant. When
/// `svg` is set, `ops` is empty and the receiver reloads instead of replaying. See [`is_uid_op`].
#[derive(Clone, Serialize, Deserialize)]
pub struct SyncMsg {
    #[serde(rename = "clientId")]
    pub client_id: String,
    pub ops: Vec<serde_json::Value>,
    /// Authoritative SVG for a full resync (structural ops); `None` = replay `ops`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub svg: Option<String>,
}

/// True for ops that address existing tree nodes by `uid` — these can't be replayed across clients
/// (uids don't match), so they trigger a full-SVG resync instead. Adds/booleans aren't here: they
/// only *create* nodes, so they replay fine (the fresh uids differ per client but never surface,
/// because any later uid-op resyncs anyway).
fn is_uid_op(ty: &str) -> bool {
    matches!(
        ty,
        "groupNodes"
            | "ungroupNode"
            | "reorderNode"
            | "moveTreeNode"
            | "setNodeHidden"
            | "setNodeBoolean"
            | "setNodeAttr"
            | "setNodeText"
    )
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
    editor.load_source(&project.svg)?;
    let (tx, _rx) = broadcast::channel(256);
    let session = Arc::new(Mutex::new(ProjectSession {
        project_id,
        editor,
        tx,
    }));
    sessions.lock().unwrap().insert(project_id, session.clone());
    Ok(session)
}

/// Apply a batch of JSON ops to a project session: mutate the editor (one undo step), broadcast to
/// the other subscribers, and persist the new SVG to SQLite. Returns how many ops applied. Sync (so
/// the MCP tools can call it directly); the DB write is spawned onto the runtime.
pub fn apply_ops(
    session: &Arc<Mutex<ProjectSession>>,
    pool: &SqlitePool,
    ops: Vec<serde_json::Value>,
    origin: &str,
) -> Result<usize, String> {
    let parsed: Vec<Op> = ops
        .iter()
        .map(|v| serde_json::from_value(v.clone()).map_err(|e| format!("invalid op: {e}")))
        .collect::<Result<_, _>>()?;
    let (svg, id, applied) = {
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
        let svg = s.editor.to_svg();
        // Broadcast to the project's other subscribers (browser + other MCP connections). A batch
        // touching tree structure by uid can't be replayed on a peer (uids differ) — ship the
        // authoritative svg for a resync instead; everything else replays by index.
        let structural = ops.iter().any(|o| {
            o.get("type")
                .and_then(|t| t.as_str())
                .is_some_and(is_uid_op)
        });
        let msg = if structural {
            SyncMsg {
                client_id: origin.to_string(),
                ops: Vec::new(),
                svg: Some(svg.clone()),
            }
        } else {
            SyncMsg {
                client_id: origin.to_string(),
                ops,
                svg: None,
            }
        };
        let _ = s.tx.send(msg);
        (svg, s.project_id, applied)
    };
    // Persist the new source (fire-and-forget; the editor is the live authority meanwhile).
    let pool = pool.clone();
    tokio::spawn(async move {
        let _ = db::update_project_svg(&pool, id, &svg).await;
    });
    Ok(applied)
}

/// Adopt a whole-document SVG snapshot from a client — the other half of the structural-op sync: a
/// browser that grouped/reordered ships the authoritative SVG (it can't express the change as
/// replayable uid-ops for peers). Load it into the session editor, broadcast it for the other
/// subscribers to resync, and persist. Errors if the markup won't parse (the session is untouched).
pub fn apply_svg(
    session: &Arc<Mutex<ProjectSession>>,
    pool: &SqlitePool,
    svg: &str,
    origin: &str,
) -> Result<(), String> {
    let (canonical, id) = {
        let mut s = session.lock().unwrap();
        s.editor.load_source(svg)?; // rebuild the authoritative tree from the peer's snapshot
        let canonical = s.editor.to_svg();
        let _ = s.tx.send(SyncMsg {
            client_id: origin.to_string(),
            ops: Vec::new(),
            svg: Some(canonical.clone()),
        });
        (canonical, s.project_id)
    };
    let pool = pool.clone();
    tokio::spawn(async move {
        let _ = db::update_project_svg(&pool, id, &canonical).await;
    });
    Ok(())
}
