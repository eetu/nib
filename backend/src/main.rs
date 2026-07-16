//! nib-backend (Phase C) — a rust-axum server that links `nib-core` **natively** (the same engine
//! the browser drives via WASM), serves the built SPA, and persists **projects** (SVG documents)
//! in SQLite, owned by token-authed users. Surfaces: a JSON `/api` (projects), the MCP tool surface
//! at `/mcp`, and (C2) live op-sync over WebSocket — all editing the same in-memory sessions.

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

mod auth;
mod db;
mod mcp;
mod session;
mod sync;

use auth::AuthUser;
use session::Sessions;

const BLANK_SVG: &str =
    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\">\n</svg>";

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub sessions: Sessions,
}

fn ise<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

#[derive(Serialize)]
struct Version {
    backend: String,
    core: String,
}

async fn version() -> Json<Version> {
    Json(Version {
        backend: env!("CARGO_PKG_VERSION").to_string(),
        core: nib_core::core_version(),
    })
}

#[derive(Serialize)]
struct Me {
    id: i64,
    name: String,
    /// The caller's own token — surfaced so the SPA can display it for copy/paste into an MCP client.
    token: String,
    projects: Vec<db::ProjectMeta>,
}

/// Who am I + my projects (a token check + a listing in one call for the SPA on connect).
async fn me(
    AuthUser(user): AuthUser,
    State(st): State<AppState>,
) -> Result<Json<Me>, (StatusCode, String)> {
    let projects = db::list_projects(&st.pool, user.id).await.map_err(ise)?;
    Ok(Json(Me {
        id: user.id,
        name: user.name,
        token: user.token,
        projects,
    }))
}

async fn list_projects(
    AuthUser(user): AuthUser,
    State(st): State<AppState>,
) -> Result<Json<Vec<db::ProjectMeta>>, (StatusCode, String)> {
    db::list_projects(&st.pool, user.id)
        .await
        .map(Json)
        .map_err(ise)
}

#[derive(Deserialize)]
struct NewProject {
    name: String,
    #[serde(default)]
    svg: Option<String>,
}

async fn create_project(
    AuthUser(user): AuthUser,
    State(st): State<AppState>,
    Json(body): Json<NewProject>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let svg = body.svg.unwrap_or_else(|| BLANK_SVG.to_string());
    nib_core::model::document::parse_svg(&svg).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let id = db::create_project(&st.pool, user.id, &body.name, &svg)
        .await
        .map_err(ise)?;
    Ok(Json(serde_json::json!({ "id": id, "name": body.name })))
}

async fn get_project(
    AuthUser(user): AuthUser,
    State(st): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<db::Project>, (StatusCode, String)> {
    db::get_project(&st.pool, user.id, id)
        .await
        .map_err(ise)?
        .map(Json)
        .ok_or((StatusCode::NOT_FOUND, "no such project".to_string()))
}

/// Replace a project's SVG (validated through the core parser so broken markup never persists).
async fn put_project(
    AuthUser(user): AuthUser,
    State(st): State<AppState>,
    Path(id): Path<i64>,
    body: String,
) -> Result<StatusCode, (StatusCode, String)> {
    if db::get_project(&st.pool, user.id, id)
        .await
        .map_err(ise)?
        .is_none()
    {
        return Err((StatusCode::NOT_FOUND, "no such project".to_string()));
    }
    nib_core::model::document::parse_svg(&body).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    db::update_project_svg(&st.pool, id, &body)
        .await
        .map_err(ise)?;
    Ok(StatusCode::NO_CONTENT)
}

#[tokio::main]
async fn main() {
    let db_url = std::env::var("NIB_DB").unwrap_or_else(|_| "sqlite:nib.db".to_string());
    let dist =
        PathBuf::from(std::env::var("NIB_DIST").unwrap_or_else(|_| "../frontend/dist".to_string()));
    let dev_token =
        std::env::var("NIB_DEV_TOKEN").unwrap_or_else(|_| db::DEV_TOKEN_DEFAULT.to_string());

    let pool = db::connect(&db_url).await.expect("open database");
    db::ensure_dev_user(&pool, &dev_token)
        .await
        .expect("seed dev user");

    let state = AppState {
        pool,
        sessions: session::new_sessions(),
    };

    // Serve the SPA, falling back to index.html for client-side deep links (family contract).
    let spa = ServeDir::new(&dist).fallback(ServeFile::new(dist.join("index.html")));

    // MCP tool surface (C3), nested at /mcp; each connection shares the process's project sessions.
    let mcp_state = state.clone();
    let mcp_service = StreamableHttpService::new(
        move || Ok(mcp::NibMcp::new(&mcp_state)),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    );

    let app = Router::new()
        .route("/api/version", get(version))
        .route("/api/me", get(me))
        .route("/api/projects", get(list_projects).post(create_project))
        .route("/api/projects/{id}", get(get_project).put(put_project))
        .route("/ws/projects/{id}", get(sync::ws_handler))
        .nest_service("/mcp", mcp_service)
        .fallback_service(spa)
        .layer(CorsLayer::permissive()) // dev: the :5173 SPA may call the :4321 API
        .with_state(state);

    let port: u16 = std::env::var("NIB_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(4321);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!(
        "nib-backend on http://{addr}  (db: {db_url}, dist: {})",
        dist.display()
    );
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind listener");
    axum::serve(listener, app).await.expect("serve");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn db_auth_project_session_roundtrip() {
        let path = std::env::temp_dir().join(format!("nib-test-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let pool = db::connect(&format!("sqlite:{}", path.display()))
            .await
            .unwrap();
        db::ensure_dev_user(&pool, "tkn").await.unwrap();
        db::ensure_dev_user(&pool, "tkn").await.unwrap(); // idempotent

        // auth: a bad token resolves to nobody; the seeded token to the developer.
        assert!(db::user_by_token(&pool, "nope").await.unwrap().is_none());
        let user = db::user_by_token(&pool, "tkn").await.unwrap().expect("dev");

        // create + list + get.
        let id = db::create_project(&pool, user.id, "demo", BLANK_SVG)
            .await
            .unwrap();
        let list = db::list_projects(&pool, user.id).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(
            db::get_project(&pool, user.id, id)
                .await
                .unwrap()
                .unwrap()
                .svg,
            BLANK_SVG
        );

        // ownership: a different user can't see or open it.
        db::ensure_dev_user(&pool, "other").await.unwrap();
        let other = db::user_by_token(&pool, "other").await.unwrap().unwrap();
        assert!(
            db::get_project(&pool, other.id, id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(db::list_projects(&pool, other.id).await.unwrap().is_empty());

        // open a session + apply an op → the editor mutates (a drawn <path> appears).
        let sessions = session::new_sessions();
        let sess = session::open(&pool, &sessions, user.id, id).await.unwrap();
        let op = serde_json::json!({
            "type": "addShape",
            "id": "c1",
            "spec": { "shape": "ellipse", "cx": 50, "cy": 50, "rx": 20, "ry": 20 },
            "attributes": { "fill": "#0088ff" }
        });
        assert_eq!(
            session::apply_ops(&sess, &pool, vec![op], "test").unwrap(),
            1
        );
        let edited = sess.lock().unwrap().editor.to_svg();
        assert!(edited.contains("<path"), "drawn shape emitted: {edited}");

        // persistence round-trips through the DB.
        db::update_project_svg(&pool, id, &edited).await.unwrap();
        let reloaded = db::get_project(&pool, user.id, id).await.unwrap().unwrap();
        assert_ne!(reloaded.svg, BLANK_SVG);
        assert!(reloaded.svg.contains("<path"));

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn apply_ops_broadcasts_to_subscribers() {
        let path = std::env::temp_dir().join(format!("nib-bcast-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let pool = db::connect(&format!("sqlite:{}", path.display()))
            .await
            .unwrap();
        db::ensure_dev_user(&pool, "tkn").await.unwrap();
        let user = db::user_by_token(&pool, "tkn").await.unwrap().unwrap();
        let id = db::create_project(&pool, user.id, "p", BLANK_SVG)
            .await
            .unwrap();
        let sessions = session::new_sessions();
        let sess = session::open(&pool, &sessions, user.id, id).await.unwrap();

        // Two subscribers (two live clients on the project).
        let mut rx1 = sess.lock().unwrap().tx.subscribe();
        let mut rx2 = sess.lock().unwrap().tx.subscribe();

        let op = serde_json::json!({
            "type": "addShape", "id": "r1",
            "spec": { "shape": "rect", "x0": 0, "y0": 0, "x1": 10, "y1": 10 },
            "attributes": {}
        });
        assert_eq!(
            session::apply_ops(&sess, &pool, vec![op], "clientA").unwrap(),
            1
        );

        // Both receive the op batch, tagged with the origin client so they can ignore an echo.
        let m1 = rx1.recv().await.unwrap();
        let m2 = rx2.recv().await.unwrap();
        assert_eq!(m1.client_id, "clientA");
        assert_eq!(m1.ops.len(), 1);
        assert!(m1.svg.is_none(), "a plain op replays — no svg snapshot");
        assert_eq!(m2.client_id, "clientA");

        let _ = std::fs::remove_file(&path);
    }

    // A tree-structural op (groupNodes) can't replay across clients — its `uid`s are per-client — so
    // the broadcast carries an authoritative SVG snapshot to resync from instead of the op.
    #[tokio::test]
    async fn structural_op_broadcasts_svg_snapshot() {
        let path = std::env::temp_dir().join(format!("nib-struct-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let pool = db::connect(&format!("sqlite:{}", path.display()))
            .await
            .unwrap();
        db::ensure_dev_user(&pool, "tkn").await.unwrap();
        let user = db::user_by_token(&pool, "tkn").await.unwrap().unwrap();
        let id = db::create_project(&pool, user.id, "p", BLANK_SVG)
            .await
            .unwrap();
        let sessions = session::new_sessions();
        let sess = session::open(&pool, &sessions, user.id, id).await.unwrap();

        // Two drawn shapes to group.
        for name in ["a", "b"] {
            let op = serde_json::json!({
                "type": "addShape", "id": name,
                "spec": { "shape": "rect", "x0": 0, "y0": 0, "x1": 10, "y1": 10 },
                "attributes": {}
            });
            session::apply_ops(&sess, &pool, vec![op], "t").unwrap();
        }
        // Their tree uids — what the group op addresses (and what a peer would fail to replay).
        let uids: Vec<String> = {
            let s = sess.lock().unwrap();
            let doc = s.editor.doc().unwrap();
            doc.paths
                .iter()
                .filter(|p| !p.deleted)
                .map(|p| p.uid.clone())
                .collect()
        };
        assert_eq!(uids.len(), 2);
        assert!(
            uids.iter().all(|u| !u.is_empty()),
            "drawn shapes carry tree uids"
        );

        let mut rx = sess.lock().unwrap().tx.subscribe();
        let group = serde_json::json!({
            "type": "groupNodes", "uids": uids, "uid": "grp-1", "name": "pair"
        });
        assert_eq!(
            session::apply_ops(&sess, &pool, vec![group], "clientA").unwrap(),
            1
        );

        let msg = rx.recv().await.unwrap();
        assert_eq!(msg.client_id, "clientA");
        assert!(
            msg.ops.is_empty(),
            "structural op broadcasts no replayable ops"
        );
        let svg = msg.svg.expect("structural op broadcasts an svg snapshot");
        assert!(
            svg.contains("<g"),
            "the grouped <g> is in the resync svg: {svg}"
        );
        assert!(svg.contains("pair"), "the group name is applied: {svg}");

        let _ = std::fs::remove_file(&path);
    }
}
