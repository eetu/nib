//! nib-backend (Phase C) — a rust-axum server that links `nib-core` **natively** (the same engine
//! the browser drives via WASM), serves the built SPA, and persists **projects** (SVG documents)
//! in SQLite, owned by token-authed users. Surfaces: a JSON `/api` (projects), the MCP tool surface
//! at `/mcp`, and (C2) live op-sync over WebSocket — all editing the same in-memory sessions.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::FromRef;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use axum_extra::extract::cookie::Key;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

mod auth;
mod config;
mod db;
mod fonts;
mod login;
mod mcp;
mod oidc;
mod session;
mod sync;

use auth::{AuthSession, AuthUser};
use config::Config;
use oidc::OidcLazy;
use session::Sessions;

const BLANK_SVG: &str =
    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\">\n</svg>";

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub sessions: Sessions,
    pub cfg: Arc<Config>,
    pub oidc: Arc<OidcLazy>,
    pub cookie_key: Key,
}

/// Lets handlers take a bare `SignedCookieJar` argument.
impl FromRef<AppState> for Key {
    fn from_ref(state: &AppState) -> Self {
        state.cookie_key.clone()
    }
}

fn ise<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    tracing::error!(error = %e, "request failed");
    (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
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
    email: Option<String>,
    /// The caller's own token — surfaced so the SPA can display it for copy/paste into an MCP
    /// client. This is why the endpoint is session-only: a leaked token must not be able to read
    /// itself back (nor, via `/api/token/rotate`, mint its own replacement).
    token: String,
    projects: Vec<db::ProjectMeta>,
}

/// Who am I + my projects (identity + a listing in one call for the SPA on connect).
async fn me(
    AuthSession(user): AuthSession,
    State(st): State<AppState>,
) -> Result<Json<Me>, (StatusCode, String)> {
    let projects = db::list_projects(&st.pool, user.id).await.map_err(ise)?;
    Ok(Json(Me {
        id: user.id,
        name: user.name,
        email: user.email,
        token: user.token,
        projects,
    }))
}

/// Replace the caller's bearer token. Any MCP client configured with the old one starts failing
/// immediately — that's the point of the button.
async fn rotate_token(
    AuthSession(user): AuthSession,
    State(st): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let token = db::rotate_token(&st.pool, user.id).await.map_err(ise)?;
    tracing::info!(user = user.id, "bearer token rotated");
    Ok(Json(serde_json::json!({ "token": token })))
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

#[derive(Deserialize)]
struct RenameProject {
    name: String,
}

/// Rename a project. Separate from `put_project`, which replaces the *document* — overloading one
/// verb for "new name" and "new artwork" would make an accidental rename destroy the drawing.
async fn patch_project(
    AuthUser(user): AuthUser,
    State(st): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<RenameProject>,
) -> Result<StatusCode, (StatusCode, String)> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name must not be empty".into()));
    }
    match db::rename_project(&st.pool, user.id, id, name).await {
        Ok(true) => Ok(StatusCode::NO_CONTENT),
        Ok(false) => Err((StatusCode::NOT_FOUND, "no such project".to_string())),
        Err(e) => Err(ise(e)),
    }
}

/// Delete a project, and drop its in-memory session so nothing keeps serving (or re-persisting)
/// a document whose row is gone.
async fn delete_project(
    AuthUser(user): AuthUser,
    State(st): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, (StatusCode, String)> {
    match db::delete_project(&st.pool, user.id, id).await {
        Ok(true) => {
            session::close(&st.sessions, id);
            Ok(StatusCode::NO_CONTENT)
        }
        Ok(false) => Err((StatusCode::NOT_FOUND, "no such project".to_string())),
        Err(e) => Err(ise(e)),
    }
}

/// Replace a project's document by importing a posted SVG: parse it into the native model (the
/// source of truth), then persist model + cached SVG. Broken markup never persists (BAD_REQUEST).
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
    // Parse first so broken markup is rejected before anything is touched…
    let mut editor = nib_core::Editor::new();
    editor
        .load_source(&body)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    // …then, if the project is open, make the *live session* take the import and persist what it
    // holds. Writing only the row would leave the resident editor on the old document, and its
    // next edit would overwrite the import.
    let (model, svg) = match session::replace_document(&st.sessions, id, &body) {
        Ok(Some(from_session)) => from_session,
        Ok(None) => (editor.to_model_json().unwrap_or_default(), editor.to_svg()),
        Err(e) => return Err((StatusCode::BAD_REQUEST, e)),
    };
    db::update_project(&st.pool, id, &model, &svg)
        .await
        .map_err(ise)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Build the router. Split out of `main` so tests can drive the real HTTP surface — auth included —
/// with `tower::ServiceExt::oneshot` instead of poking at `db::` functions underneath it.
pub fn app(state: AppState) -> Router {
    // Serve the SPA, falling back to index.html for client-side deep links (family contract).
    let dist = state.cfg.dist.clone();
    let spa = ServeDir::new(&dist).fallback(ServeFile::new(dist.join("index.html")));

    // MCP tool surface (C3), nested at /mcp; each connection shares the process's project sessions.
    let mcp_state = state.clone();
    let mcp_service = StreamableHttpService::new(
        move || Ok(mcp::NibMcp::new(&mcp_state)),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    );

    let router = Router::new()
        .route("/api/version", get(version))
        .route("/api/me", get(me))
        .route("/api/token/rotate", post(rotate_token))
        .route("/api/projects", get(list_projects).post(create_project))
        .route(
            "/api/projects/{id}",
            get(get_project)
                .put(put_project)
                .patch(patch_project)
                .delete(delete_project),
        )
        .route("/auth/login", get(login::login))
        .route("/auth/callback", get(login::callback))
        .route("/auth/logout", post(login::logout))
        .route("/ws/projects/{id}", get(sync::ws_handler))
        .nest_service("/mcp", mcp_service)
        .fallback_service(spa);

    // Permissive CORS exists only for the dev split (the :5173 SPA calling the :4321 API). In
    // production the SPA is same-origin, and a wide-open CORS policy alongside a session cookie is
    // how you get CSRF — so it is never applied there.
    let router = if state.cfg.dev_auth {
        router.layer(CorsLayer::permissive())
    } else {
        router
    };

    router.with_state(state)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "nib_backend=info,tower_http=warn".into()),
        )
        .init();

    let cfg = match Config::from_env() {
        Ok(c) => Arc::new(c),
        Err(e) => {
            tracing::error!("{e}");
            std::process::exit(1);
        }
    };
    if cfg.dev_auth {
        tracing::warn!("NIB_DEV_AUTH=1 — auth gate bypassed; do not use in production");
    }
    if !cfg.oidc.is_some() && !cfg.dev_auth {
        // The deploy-1 state: nib is up and healthy, but nobody can sign in until kanidm has
        // minted the client secret and the second deploy writes the OIDC_* block.
        tracing::warn!("no OIDC_* configured — sign-in is unavailable until it is");
    }

    let pool = db::connect(&cfg.db_url).await.expect("open database");
    if cfg.dev_auth {
        db::ensure_dev_user(&pool, &cfg.dev_token)
            .await
            .expect("seed dev user");
    }

    let sessions = session::new_sessions();
    session::spawn_evictor(sessions.clone(), pool.clone());

    let state = AppState {
        pool,
        sessions,
        cookie_key: Key::from(&hex::decode(&cfg.session_key).expect("session key is hex")),
        oidc: Arc::new(OidcLazy::new(cfg.oidc.clone())),
        cfg: cfg.clone(),
    };

    // Load the fonts at boot rather than on the first label: it costs milliseconds, and a runtime
    // with no faces (a `scratch` image built without the fonts stage, a mount that didn't land)
    // then says so in the startup log instead of surfacing much later as "no font found" from a
    // conversion the user was in the middle of.
    let faces = fonts::database().len();
    if faces == 0 {
        tracing::warn!("no fonts available — outlining text and rendering labels will not work");
    }

    let addr = SocketAddr::from(([127, 0, 0, 1], cfg.port));
    tracing::info!(
        "nib-backend on http://{addr} (db: {}, dist: {}, fonts: {faces})",
        cfg.db_url,
        cfg.dist.display()
    );
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind listener");
    axum::serve(listener, app(state)).await.expect("serve");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    /// A throwaway SQLite file. Named per test so the suite can run in parallel.
    async fn test_pool(tag: &str) -> (SqlitePool, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("nib-{tag}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let pool = db::connect(&format!("sqlite:{}", path.display()))
            .await
            .unwrap();
        (pool, path)
    }

    /// Production-shaped state: dev auth **off**, so the tests exercise the real credential tiers
    /// rather than falling through to the synthetic identity.
    fn test_state(pool: SqlitePool) -> AppState {
        let session_key = config::random_hex(64);
        let cfg = Arc::new(Config {
            db_url: String::new(),
            dist: std::env::temp_dir(),
            port: 0,
            dev_auth: false,
            dev_token: String::new(),
            session_key: session_key.clone(),
            oidc: None,
        });
        AppState {
            pool,
            sessions: session::new_sessions(),
            cookie_key: Key::from(&hex::decode(&session_key).unwrap()),
            oidc: Arc::new(OidcLazy::new(None)),
            cfg,
        }
    }

    async fn get(state: &AppState, uri: &str, token: Option<&str>) -> (StatusCode, String) {
        send(state, Request::builder().method("GET").uri(uri), token).await
    }

    async fn send(
        state: &AppState,
        req: axum::http::request::Builder,
        token: Option<&str>,
    ) -> (StatusCode, String) {
        let req = match token {
            Some(t) => req.header("authorization", format!("Bearer {t}")),
            None => req,
        };
        let res = app(state.clone())
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&body).to_string())
    }

    #[tokio::test]
    async fn db_auth_project_session_roundtrip() {
        let (pool, path) = test_pool("test").await;
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
        let other = db::resolve_user(&pool, "sub-other", "other@example.com", "other")
            .await
            .unwrap();
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

        // persistence round-trips through the DB (model = source of truth, svg = cached export).
        let model = sess.lock().unwrap().editor.to_model_json().unwrap();
        db::update_project(&pool, id, &model, &edited)
            .await
            .unwrap();
        let reloaded = db::get_project(&pool, user.id, id).await.unwrap().unwrap();
        assert!(!reloaded.model.is_empty(), "model persisted");
        assert_ne!(reloaded.svg, BLANK_SVG);
        assert!(reloaded.svg.contains("<path"));

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn apply_ops_broadcasts_to_subscribers() {
        let (pool, path) = test_pool("bcast").await;
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
        // The create-op is broadcast carrying a minted uid (the caller supplied none), so every
        // peer replaying it agrees on the new node's identity instead of inventing its own.
        assert!(
            m1.ops[0]
                .get("uid")
                .and_then(|v| v.as_str())
                .is_some_and(|u| !u.is_empty()),
            "create-op broadcast carries a uid: {:?}",
            m1.ops[0]
        );
        assert_eq!(m2.client_id, "clientA");

        let _ = std::fs::remove_file(&path);
    }

    // A tree-structural op (groupNodes) now replays as a plain op on every client: all clients load
    // the same native model, so the origin's node uids resolve identically on a peer (no snapshot).
    #[tokio::test]
    async fn structural_op_replays_as_op() {
        let (pool, path) = test_pool("struct").await;
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
        // Their tree uids — carried by the shared model, so the op replays identically on a peer.
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

        // The structural op is broadcast verbatim as an op (peers replay it against their identical
        // model), and it actually grouped in the authoritative editor.
        let msg = rx.recv().await.unwrap();
        assert_eq!(msg.client_id, "clientA");
        assert_eq!(msg.ops.len(), 1, "structural op replays as an op");
        assert_eq!(msg.ops[0]["type"], "groupNodes");
        let svg = sess.lock().unwrap().editor.to_svg();
        assert!(
            svg.contains("<g"),
            "grouped in the authoritative editor: {svg}"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// The bearer token is an *API* credential: good enough to drive projects (that's what an MCP
    /// client does), never good enough to read itself back or mint its replacement.
    #[tokio::test]
    async fn bearer_drives_the_api_but_not_the_account() {
        let (pool, path) = test_pool("bearer").await;
        let user = db::resolve_user(&pool, "sub-a", "a@example.com", "a")
            .await
            .unwrap();
        let st = test_state(pool);

        let (status, _) = get(&st, "/api/projects", Some(&user.token)).await;
        assert_eq!(status, StatusCode::OK, "bearer drives the project API");

        let (status, _) = get(&st, "/api/me", Some(&user.token)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "bearer can't read itself");

        let (status, _) = send(
            &st,
            Request::builder().method("POST").uri("/api/token/rotate"),
            Some(&user.token),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "bearer can't mint its own replacement"
        );

        // …and no credential at all is a 401, not a fallthrough.
        let (status, _) = get(&st, "/api/projects", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn rotate_invalidates_the_previous_token() {
        let (pool, path) = test_pool("rotate").await;
        let user = db::resolve_user(&pool, "sub-a", "a@example.com", "a")
            .await
            .unwrap();
        let old = user.token.clone();

        let fresh = db::rotate_token(&pool, user.id).await.unwrap();
        assert_ne!(fresh, old);

        let st = test_state(pool.clone());
        let (status, _) = get(&st, "/api/projects", Some(&old)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "old token is dead");
        let (status, _) = get(&st, "/api/projects", Some(&fresh)).await;
        assert_eq!(status, StatusCode::OK, "new token works");

        let _ = std::fs::remove_file(&path);
    }

    /// Regression: `session::open` used to check ownership only on the cold path, so once a
    /// project was resident any authenticated user could attach to it. Every surface — the
    /// WebSocket, MCP `open_project`, and every MCP tool via `active_session` — funnels through
    /// this one call, so verifying it here covers all three.
    #[tokio::test]
    async fn a_cached_session_still_refuses_a_foreign_user() {
        let (pool, path) = test_pool("tenancy").await;
        let owner = db::resolve_user(&pool, "sub-owner", "owner@example.com", "owner")
            .await
            .unwrap();
        let intruder = db::resolve_user(&pool, "sub-intruder", "x@example.com", "x")
            .await
            .unwrap();
        let id = db::create_project(&pool, owner.id, "private", BLANK_SVG)
            .await
            .unwrap();

        // The owner opens it — the project is now resident in the registry.
        let sessions = session::new_sessions();
        assert!(
            session::open(&pool, &sessions, owner.id, id).await.is_ok(),
            "owner can open their own project"
        );

        // The intruder hits the warm cache. This is the exact path that used to succeed.
        assert!(
            session::open(&pool, &sessions, intruder.id, id)
                .await
                .is_err(),
            "a cached session must not be attachable by a non-owner"
        );

        // …and the REST surface agrees, indistinguishably from a missing project.
        let st = AppState {
            sessions,
            ..test_state(pool)
        };
        let (status, _) = get(&st, &format!("/api/projects/{id}"), Some(&intruder.token)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let _ = std::fs::remove_file(&path);
    }

    /// Rename + delete are scoped in the SQL itself, so a non-owner gets the same 404 as for a
    /// project that doesn't exist — and deleting drops the resident session rather than leaving an
    /// `Editor` serving a row that's gone.
    #[tokio::test]
    async fn projects_can_be_renamed_and_deleted_by_their_owner_only() {
        let (pool, path) = test_pool("crud").await;
        let owner = db::resolve_user(&pool, "sub-owner", "owner@example.com", "owner")
            .await
            .unwrap();
        let intruder = db::resolve_user(&pool, "sub-intruder", "x@example.com", "x")
            .await
            .unwrap();
        let id = db::create_project(&pool, owner.id, "sketch", BLANK_SVG)
            .await
            .unwrap();
        let st = test_state(pool.clone());

        let rename = |token: String, name: &str| {
            let body = serde_json::json!({ "name": name }).to_string();
            let st = st.clone();
            async move {
                let res = app(st)
                    .oneshot(
                        Request::builder()
                            .method("PATCH")
                            .uri(format!("/api/projects/{id}"))
                            .header("authorization", format!("Bearer {token}"))
                            .header("content-type", "application/json")
                            .body(Body::from(body))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                res.status()
            }
        };

        assert_eq!(
            rename(intruder.token.clone(), "stolen").await,
            StatusCode::NOT_FOUND,
            "a non-owner can't rename"
        );
        assert_eq!(
            rename(owner.token.clone(), "  final  ").await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            db::get_project(&pool, owner.id, id)
                .await
                .unwrap()
                .unwrap()
                .name,
            "final",
            "the stored name is trimmed"
        );
        assert_eq!(
            rename(owner.token.clone(), "   ").await,
            StatusCode::BAD_REQUEST,
            "an all-whitespace name is refused, not stored"
        );

        // Open it so there's a resident session to evict.
        session::open(&pool, &st.sessions, owner.id, id)
            .await
            .unwrap();
        assert!(st.sessions.lock().unwrap().contains_key(&id));

        let del = |token: String| {
            let st = st.clone();
            async move {
                let res = app(st)
                    .oneshot(
                        Request::builder()
                            .method("DELETE")
                            .uri(format!("/api/projects/{id}"))
                            .header("authorization", format!("Bearer {token}"))
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                res.status()
            }
        };

        assert_eq!(
            del(intruder.token).await,
            StatusCode::NOT_FOUND,
            "a non-owner can't delete"
        );
        assert!(
            db::get_project(&pool, owner.id, id)
                .await
                .unwrap()
                .is_some(),
            "…and the project survives the attempt"
        );

        assert_eq!(del(owner.token).await, StatusCode::NO_CONTENT);
        assert!(
            db::get_project(&pool, owner.id, id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            !st.sessions.lock().unwrap().contains_key(&id),
            "the resident session is dropped with the row"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// Importing an SVG into an *open* project has to reach the live session, not just the row.
    /// It used to write only the DB, so a browser (or the LLM over MCP) went on editing the
    /// previous document and its next edit persisted that back over the import.
    #[tokio::test]
    async fn importing_into_an_open_project_updates_the_live_session() {
        let (pool, path) = test_pool("import").await;
        let user = db::resolve_user(&pool, "sub-a", "a@example.com", "a")
            .await
            .unwrap();
        let id = db::create_project(&pool, user.id, "target", BLANK_SVG)
            .await
            .unwrap();
        let st = test_state(pool.clone());

        // Open it, as a browser or an MCP connection would, then subscribe like a peer.
        let sess = session::open(&pool, &st.sessions, user.id, id)
            .await
            .unwrap();
        let mut rx = sess.lock().unwrap().tx.subscribe();
        assert!(!sess.lock().unwrap().editor.to_svg().contains("circle"));

        // r##…##: the colour literal contains `"#`, which would close an `r#"…"#` string early.
        let imported = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><circle cx="50" cy="50" r="30" fill="#e11"/></svg>"##;
        let res = app(st.clone())
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/projects/{id}"))
                    .header("authorization", format!("Bearer {}", user.token))
                    .body(Body::from(imported))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NO_CONTENT);

        // The resident editor now holds the import…
        assert!(
            sess.lock().unwrap().editor.to_svg().contains("circle"),
            "the live session took the import"
        );
        // …the row agrees…
        let stored = db::get_project(&pool, user.id, id).await.unwrap().unwrap();
        assert!(stored.svg.contains("circle"), "persisted: {}", stored.svg);
        assert!(!stored.model.is_empty(), "model persisted too");
        // …and peers are told to re-fetch, since a whole-document swap isn't expressible as ops.
        let msg = rx.recv().await.unwrap();
        assert!(msg.reload, "peers get a reload signal");
        assert!(msg.ops.is_empty(), "and no ops to replay");

        let _ = std::fs::remove_file(&path);
    }

    /// The pre-registration state: no `OIDC_*` yet, kanidm hasn't issued the client secret. nib
    /// must stay up and answer honestly rather than crash or let anyone in.
    #[tokio::test]
    async fn without_oidc_the_service_stays_up_and_closed() {
        let (pool, path) = test_pool("nooidc").await;
        let st = test_state(pool);

        let (status, _) = get(&st, "/api/version", None).await;
        assert_eq!(status, StatusCode::OK, "health check is unauthenticated");

        let (status, body) = get(&st, "/auth/login", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("auth not configured"), "actionable: {body}");

        let (status, _) = get(&st, "/api/me", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let _ = std::fs::remove_file(&path);
    }

    /// A database from before OIDC carries a `developer` row whose bearer is the published
    /// `nib-dev-token`, seeded on every boot by that build. Migration 0004 retires it; this checks
    /// both halves of the outcome — the known credential stops working, and the row keeps its id
    /// (and therefore its projects).
    #[tokio::test]
    async fn the_shared_dev_token_is_retired_from_a_pre_oidc_row() {
        let (pool, path) = test_pool("olddev").await;
        // Stand in for the pre-OIDC seed: a row with no `sub`, holding the shared token. (The
        // migrations have already run, so this is written the way that build left it.)
        sqlx::query("insert into users (name, token) values ('developer', ?)")
            .bind(db::DEV_TOKEN_DEFAULT)
            .execute(&pool)
            .await
            .unwrap();
        // …and re-run the migration that retires it, since it landed before this row existed.
        sqlx::query(
            "update users set token = 'nib_' || lower(hex(randomblob(32))) \
             where token = ? and (sub is null or sub <> 'dev')",
        )
        .bind(db::DEV_TOKEN_DEFAULT)
        .execute(&pool)
        .await
        .unwrap();

        assert!(
            db::user_by_token(&pool, db::DEV_TOKEN_DEFAULT)
                .await
                .unwrap()
                .is_none(),
            "the published token no longer resolves to anyone"
        );

        // Seeding the dev user now succeeds where it used to hit the unique index and panic, and it
        // claims the token for the real `sub = 'dev'` identity.
        let dev = db::ensure_dev_user(&pool, db::DEV_TOKEN_DEFAULT)
            .await
            .unwrap();
        let by_token = db::user_by_token(&pool, db::DEV_TOKEN_DEFAULT)
            .await
            .unwrap()
            .expect("the dev token resolves again");
        assert_eq!(by_token.id, dev.id);
        // Twice, because a dev restarts the backend all day.
        db::ensure_dev_user(&pool, db::DEV_TOKEN_DEFAULT)
            .await
            .unwrap();

        // The old row is still there — its projects were never anyone else's to take.
        let rows: i64 = sqlx::query_scalar("select count(*) from users")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 2, "the pre-OIDC row and the dev identity coexist");

        pool.close().await;
        let _ = std::fs::remove_file(&path);
    }

    /// The seed also has to survive the state that caused the panic in the first place: something
    /// *else* holding the token it is about to claim.
    #[tokio::test]
    async fn seeding_takes_the_dev_token_from_whoever_holds_it() {
        let (pool, path) = test_pool("takedev").await;
        let other = db::resolve_user(&pool, "someone-else", "them@example.com", "them")
            .await
            .unwrap();
        db::set_token(&pool, other.id, db::DEV_TOKEN_DEFAULT)
            .await
            .unwrap();

        let dev = db::ensure_dev_user(&pool, db::DEV_TOKEN_DEFAULT)
            .await
            .unwrap();
        let holder = db::user_by_token(&pool, db::DEV_TOKEN_DEFAULT)
            .await
            .unwrap()
            .expect("someone holds it");
        assert_eq!(holder.id, dev.id, "the dev identity ends up with it");
        // The other user keeps working — with a token of their own, not a shared one.
        let them = db::user_by_sub(&pool, "someone-else")
            .await
            .unwrap()
            .expect("still there");
        assert_eq!(them.id, other.id);
        assert_ne!(them.token, db::DEV_TOKEN_DEFAULT);
        assert!(them.token.starts_with("nib_"));

        pool.close().await;
        let _ = std::fs::remove_file(&path);
    }
}
