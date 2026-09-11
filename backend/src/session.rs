//! In-memory project sessions (Phase C). Each open project has ONE authoritative `nib_core::Editor`
//! keyed by project id, shared by every client editing it (the MCP connection + the browser over
//! WebSocket). All edits funnel through [`apply_ops`]: mutate the editor, broadcast the ops to the
//! project's other subscribers, and persist the native model (+ a cached SVG export) to SQLite.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
    /// Set when the whole document was replaced (an import through `PUT /api/projects/{id}`)
    /// rather than edited. Ops can't express that, so peers re-fetch the project instead of
    /// replaying anything.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reload: bool,
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
        | "stampInstance" | "setDropShadow" | "addText"
            if op.get("uid").and_then(|v| v.as_str()).is_none() =>
        {
            op["uid"] = serde_json::json!(nib_core::model::tree::new_id());
        }
        _ => {}
    }
}

/// The authoritative in-memory session for one open project.
pub struct ProjectSession {
    pub project_id: i64,
    pub editor: Editor,
    pub tx: broadcast::Sender<SyncMsg>,
    /// When this session was last opened or edited — drives idle eviction.
    pub last_touched: Instant,
}

/// Get (or lazily load from the DB) the session for a project the `user_id` owns.
///
/// **Ownership is checked before the cache**, deliberately. The check used to sit *after* the
/// cache lookup, which meant only the first caller to open a project was verified: once it was
/// resident, any authenticated user could attach to it — read and write — for the lifetime of the
/// process, via the WebSocket, MCP `open_project`, or any MCP tool. Invisible with a single
/// seeded user; a cross-tenant breach the moment real users exist. It's one indexed read.
pub async fn open(
    pool: &SqlitePool,
    sessions: &Sessions,
    user_id: i64,
    project_id: i64,
) -> Result<Arc<Mutex<ProjectSession>>, String> {
    let project = db::get_project(pool, user_id, project_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no such project: {project_id}"))?;
    if let Some(s) = sessions.lock().unwrap().get(&project_id).cloned() {
        s.lock().unwrap().last_touched = Instant::now();
        return Ok(s);
    }
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
        last_touched: Instant::now(),
    }));
    sessions.lock().unwrap().insert(project_id, session.clone());
    Ok(session)
}

/// Replace a live session's whole document with freshly imported source.
///
/// Without this, `PUT /api/projects/{id}` wrote the database while the resident `Editor` kept the
/// *old* document — so an open browser (or the LLM over MCP) went on editing the previous
/// drawing, and the next `apply_ops` persisted that back over the import. Returns the new
/// (model, svg) when a session was live, so the caller persists exactly what the session now
/// holds; `None` when the project isn't open and the caller should persist its own parse.
///
/// Peers are told to re-fetch rather than sent ops: a whole-document swap mints new node uids, so
/// there is nothing for them to replay against.
pub fn replace_document(
    sessions: &Sessions,
    project_id: i64,
    source: &str,
) -> Result<Option<(String, String)>, String> {
    let Some(sess) = sessions.lock().unwrap().get(&project_id).cloned() else {
        return Ok(None);
    };
    let mut s = sess.lock().unwrap();
    s.editor.load_source(source)?;
    s.last_touched = Instant::now();
    let model = s.editor.to_model_json().unwrap_or_default();
    let svg = s.editor.to_svg();
    let _ = s.tx.send(SyncMsg {
        client_id: String::new(),
        ops: Vec::new(),
        reload: true,
    });
    Ok(Some((model, svg)))
}

/// Drop a project's in-memory session, if it has one.
///
/// Called when the project is deleted: the resident `Editor` would otherwise keep serving the
/// document to anyone still attached, and the idle sweep would try to flush it back to a row that
/// no longer exists. Any live WebSocket sees its broadcast channel close and disconnects.
pub fn close(sessions: &Sessions, project_id: i64) {
    sessions.lock().unwrap().remove(&project_id);
}

/// How long a project with no live subscribers stays resident before being dropped.
const IDLE_EVICT: Duration = Duration::from_secs(15 * 60);
const SWEEP_EVERY: Duration = Duration::from_secs(60);

/// Periodically drop idle sessions.
///
/// Nothing used to leave the registry, so every project ever opened kept a whole `Editor` resident
/// for the process lifetime — on a memory-capped Pi that's what eventually OOM-restarts the unit.
/// A session is evictable when it has no broadcast subscribers (no WebSocket attached) and hasn't
/// been touched recently; its model is flushed once more on the way out, since `apply_ops`
/// persists via a detached task that may not have landed.
pub fn spawn_evictor(sessions: Sessions, pool: SqlitePool) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        loop {
            tick.tick().await;
            let stale: Vec<(i64, String, String)> = {
                let mut map = sessions.lock().unwrap();
                let mut drained = Vec::new();
                map.retain(|&id, sess| {
                    let s = sess.lock().unwrap();
                    let idle = s.tx.receiver_count() == 0 && s.last_touched.elapsed() > IDLE_EVICT;
                    if idle {
                        drained.push((
                            id,
                            s.editor.to_model_json().unwrap_or_default(),
                            s.editor.to_svg(),
                        ));
                    }
                    !idle
                });
                drained
            };
            for (id, model, svg) in stale {
                tracing::debug!(project = id, "evicting idle project session");
                let _ = db::update_project(&pool, id, &model, &svg).await;
            }
        }
    });
}

/// Apply a batch of JSON ops to a project session: mutate the editor (one undo step), broadcast the
/// ops to the other subscribers, and persist the native model (+ cached SVG) to SQLite. Returns how
/// many ops applied. Sync (so the MCP tools can call it directly); the DB write is spawned.
/// Step the document's undo history, then tell every client to reload.
///
/// Undo cannot be broadcast as an op. Peers replay ops against their own copy, and "undo" is a
/// statement about *this* document's history rather than a change anyone else can replay — so the
/// sync message carries `reload` and clients re-read the model, the same door a wholesale document
/// replacement goes through. Note the history is the DOCUMENT's, shared with whoever else is
/// editing: this can take back their step, not only the caller's.
pub fn step_history(
    session: &Arc<Mutex<ProjectSession>>,
    pool: &SqlitePool,
    steps: usize,
    undo: bool,
) -> Result<usize, String> {
    let (model, svg, id, done) = {
        let mut s = session.lock().unwrap();
        let mut done = 0usize;
        for _ in 0..steps.clamp(1, 100) {
            let stepped = if undo {
                s.editor.undo()
            } else {
                s.editor.redo()
            };
            if !stepped {
                break; // ran out of history — report how far it got
            }
            done += 1;
        }
        if done == 0 {
            return Ok(0);
        }
        s.last_touched = Instant::now();
        let model = s.editor.to_model_json().unwrap_or_default();
        let svg = s.editor.to_svg();
        let _ = s.tx.send(SyncMsg {
            client_id: "mcp".to_string(),
            ops: Vec::new(),
            reload: true,
        });
        (model, svg, s.project_id, done)
    };
    let pool = pool.clone();
    tokio::spawn(async move {
        let _ = db::update_project(&pool, id, &model, &svg).await;
    });
    Ok(done)
}

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
        s.last_touched = Instant::now();
        s.editor.commit();
        // The native model is the source of truth (persisted); the svg is a cached export.
        let model = s.editor.to_model_json().unwrap_or_default();
        let svg = s.editor.to_svg();
        // Broadcast the ops to the project's other subscribers. All clients load the same model
        // (identical node uids), so every op — structural uid-ops included — replays correctly.
        let _ = s.tx.send(SyncMsg {
            client_id: origin.to_string(),
            ops,
            reload: false,
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
