//! nib-backend (Phase C) — a rust-axum server that links `nib-core` **natively** (the same engine
//! the browser drives via WASM), serves the built SPA, and persists **projects** (SVG documents)
//! in SQLite, owned by token-authed users. Surfaces: a JSON `/api` (projects), the MCP tool surface
//! at `/mcp`, and (C2) live op-sync over WebSocket — all editing the same in-memory sessions.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::FromRef;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::StatusCode,
    routing::{get, post},
};
use axum_extra::extract::cookie::Key;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

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
    /// The first characters of the caller's token, so the SPA can say *which* token a client is
    /// configured with. Not the token: only its hash is stored, so nothing can hand it back after
    /// the moment it was minted — `/api/token/rotate` returns a new one once, and that is the only
    /// time it exists outside the caller. The endpoint stays session-only anyway, since a leaked
    /// token must not be able to mint its replacement.
    #[serde(rename = "tokenHint")]
    token_hint: String,
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
        token_hint: user.token_hint,
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
///
/// **`If-Match` carries the generation the caller believes it is replacing** (see
/// `migrations/0005`). Sent and stale → `409` with the current one, so a second import can't land
/// on top of one its author never saw. Absent → forced, which is what every pre-`If-Match` client
/// does and what a deliberate overwrite looks like.
async fn put_project(
    AuthUser(user): AuthUser,
    State(st): State<AppState>,
    Path(id): Path<i64>,
    headers: axum::http::HeaderMap,
    body: String,
) -> Result<StatusCode, (StatusCode, String)> {
    let Some(project) = db::get_project(&st.pool, user.id, id).await.map_err(ise)? else {
        return Err((StatusCode::NOT_FOUND, "no such project".to_string()));
    };
    // A malformed If-Match is a caller bug, not a licence to force the write.
    let expected = match headers.get(axum::http::header::IF_MATCH) {
        None => None,
        Some(v) => Some(
            v.to_str()
                .ok()
                .and_then(|s| s.trim_matches('"').parse::<i64>().ok())
                .ok_or((
                    StatusCode::BAD_REQUEST,
                    "If-Match must be the project's generation, an integer".to_string(),
                ))?,
        ),
    };
    if expected.is_some_and(|g| g != project.generation) {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "the project moved on: it is at generation {} — re-open it and redo the import",
                project.generation
            ),
        ));
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
    // Conditional in SQL, so the read above narrowing to "not stale" and this write are not a
    // window another importer can slip through: two racing callers both pass the check, only one
    // passes the update.
    if db::replace_project(&st.pool, id, &model, &svg, expected)
        .await
        .map_err(ise)?
        .is_none()
    {
        return Err((
            StatusCode::CONFLICT,
            "the project was replaced while this import was in flight — re-open it and redo the \
             import"
                .to_string(),
        ));
    }
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

    // Request-scoped limits belong on the REST surface ONLY. A global timeout would cut the
    // WebSocket and MCP's Streamable-HTTP stream, both of which are long-lived by design — the
    // co-editing session IS the connection staying open.
    let api = Router::new()
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
        // A `PUT` carries a whole SVG. Axum's 2MB default rejects a real illustration with a bare
        // 413 and no hint, so the cap is raised to something a drawing can actually hit — but kept,
        // because an unbounded body is a memory-exhaustion lever on a 256MB container.
        .layer(DefaultBodyLimit::max(16 * 1024 * 1024))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(30),
        ));

    let router = api
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

    // Outermost, so it covers every route including the SPA and MCP. A panic in one handler
    // becomes a 500 for that request instead of a dropped connection with no status and no log
    // line — and, with `session::lock` recovering poisoned guards, the project stays usable after.
    router
        .layer(CatchPanicLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
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
        let edited = session::lock(&sess).editor.to_svg();
        assert!(edited.contains("<path"), "drawn shape emitted: {edited}");

        // persistence round-trips through the DB (model = source of truth, svg = cached export).
        let model = session::lock(&sess).editor.to_model_json().unwrap();
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
        let mut rx1 = session::lock(&sess).tx.subscribe();
        let mut rx2 = session::lock(&sess).tx.subscribe();

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
            let s = session::lock(&sess);
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

        let mut rx = session::lock(&sess).tx.subscribe();
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
        let svg = session::lock(&sess).editor.to_svg();
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
        let token = db::rotate_token(&pool, user.id).await.unwrap();
        let st = test_state(pool);

        let (status, _) = get(&st, "/api/projects", Some(&token)).await;
        assert_eq!(status, StatusCode::OK, "bearer drives the project API");

        let (status, _) = get(&st, "/api/me", Some(&token)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "bearer can't read itself");

        let (status, _) = send(
            &st,
            Request::builder().method("POST").uri("/api/token/rotate"),
            Some(&token),
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
        let old = db::rotate_token(&pool, user.id).await.unwrap();

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
        let intruder_token = db::rotate_token(&pool, intruder.id).await.unwrap();
        let st = AppState {
            sessions,
            ..test_state(pool)
        };
        let (status, _) = get(&st, &format!("/api/projects/{id}"), Some(&intruder_token)).await;
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
        let owner_token = db::rotate_token(&pool, owner.id).await.unwrap();
        let intruder_token = db::rotate_token(&pool, intruder.id).await.unwrap();
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
            rename(intruder_token.clone(), "stolen").await,
            StatusCode::NOT_FOUND,
            "a non-owner can't rename"
        );
        assert_eq!(
            rename(owner_token.clone(), "  final  ").await,
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
            rename(owner_token.clone(), "   ").await,
            StatusCode::BAD_REQUEST,
            "an all-whitespace name is refused, not stored"
        );

        // Open it so there's a resident session to evict.
        session::open(&pool, &st.sessions, owner.id, id)
            .await
            .unwrap();
        assert!(session::lock(&st.sessions).contains_key(&id));

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
            del(intruder_token).await,
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

        assert_eq!(del(owner_token).await, StatusCode::NO_CONTENT);
        assert!(
            db::get_project(&pool, owner.id, id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            !session::lock(&st.sessions).contains_key(&id),
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
        let token = db::rotate_token(&pool, user.id).await.unwrap();
        let mut rx = session::lock(&sess).tx.subscribe();
        assert!(!session::lock(&sess).editor.to_svg().contains("circle"));

        // r##…##: the colour literal contains `"#`, which would close an `r#"…"#` string early.
        let imported = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><circle cx="50" cy="50" r="30" fill="#e11"/></svg>"##;
        let res = app(st.clone())
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/projects/{id}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(imported))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NO_CONTENT);

        // The resident editor now holds the import…
        assert!(
            session::lock(&sess).editor.to_svg().contains("circle"),
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

    /// Two imports racing: the second must lose loudly rather than silently win.
    ///
    /// Ops can't produce this — one authoritative session applies them in order — so whole-document
    /// replacement is the only lost-update hazard, and `generation` + `If-Match` is the guard.
    #[tokio::test]
    async fn a_stale_import_is_refused_instead_of_clobbering() {
        async fn generation_of(st: AppState, token: &str, id: i64) -> i64 {
            let res = app(st)
                .oneshot(
                    Request::builder()
                        .uri(format!("/api/projects/{id}"))
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let body = axum::body::to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap();
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["generation"]
                .as_i64()
                .expect("the project carries a generation")
        }

        async fn import(
            st: AppState,
            token: &str,
            id: i64,
            svg: &str,
            if_match: Option<&str>,
        ) -> StatusCode {
            let mut req = Request::builder()
                .method("PUT")
                .uri(format!("/api/projects/{id}"))
                .header("authorization", format!("Bearer {token}"));
            if let Some(g) = if_match {
                req = req.header("if-match", g);
            }
            app(st)
                .oneshot(req.body(Body::from(svg.to_string())).unwrap())
                .await
                .unwrap()
                .status()
        }

        let (pool, path) = test_pool("conflict").await;
        let user = db::resolve_user(&pool, "sub-a", "a@example.com", "a")
            .await
            .unwrap();
        let id = db::create_project(&pool, user.id, "target", BLANK_SVG)
            .await
            .unwrap();
        let st = test_state(pool.clone());
        let token = db::rotate_token(&pool, user.id).await.unwrap();

        // Both clients read the same generation — the state a race starts from.
        let seen = generation_of(st.clone(), &token, id).await;

        // r##…##: the colour literals contain `"#`, which would close an `r#"…"#` string early.
        let first = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><circle cx="50" cy="50" r="30" fill="#e11"/></svg>"##;
        let second = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="10" y="10" width="20" height="20" fill="#11e"/></svg>"##;
        let stale = seen.to_string();

        assert_eq!(
            import(st.clone(), &token, id, first, Some(&stale)).await,
            StatusCode::NO_CONTENT
        );
        // The loser is told, and — the point — the winner's document is still there.
        assert_eq!(
            import(st.clone(), &token, id, second, Some(&stale)).await,
            StatusCode::CONFLICT
        );
        let stored = db::get_project(&pool, user.id, id).await.unwrap().unwrap();
        assert!(
            stored.svg.contains("circle"),
            "first import survived: {}",
            stored.svg
        );
        assert!(!stored.svg.contains("rect"), "stale import did not land");
        assert_eq!(stored.generation, seen + 1, "one replacement, one bump");

        // Re-read and retry: the same import now succeeds, so the guard is recoverable, not a wall.
        let fresh = generation_of(st.clone(), &token, id).await.to_string();
        assert_eq!(
            import(st.clone(), &token, id, second, Some(&fresh)).await,
            StatusCode::NO_CONTENT
        );
        assert!(
            db::get_project(&pool, user.id, id)
                .await
                .unwrap()
                .unwrap()
                .svg
                .contains("rect"),
            "the retry landed"
        );

        // No If-Match at all still forces, which is what every pre-If-Match client does.
        assert_eq!(
            import(st.clone(), &token, id, first, None).await,
            StatusCode::NO_CONTENT
        );
        // A malformed one is a caller bug, not a licence to force.
        assert_eq!(
            import(st.clone(), &token, id, first, Some("\"not-a-number\"")).await,
            StatusCode::BAD_REQUEST
        );

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

    /// A migration has to apply to a database that already holds DATA, not just to a fresh file.
    ///
    /// This is the test whose absence let a broken migration reach the Pi. Every other test starts
    /// from an empty database, where `projects` has no rows — so 0006's original table rebuild
    /// dropped `users` without violating `projects.user_id`'s foreign key, passed everywhere, and
    /// then crash-looped the deployment on the first real row. Foreign keys are on (`db::connect`),
    /// which is what turned a latent rebuild into a hard failure.
    ///
    /// It drives the migration FILES rather than `db::connect`, because what has to be proven is
    /// that the SQL survives existing rows — and `_sqlx_migrations` bookkeeping would only get in
    /// the way of standing a pre-0006 database up by hand.
    #[tokio::test]
    async fn a_migration_applies_to_a_database_that_already_has_rows() {
        let path = std::env::temp_dir().join(format!("nib-mig-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .unwrap();
        // One connection for the whole test: `pragma foreign_keys` is per-connection, and the
        // point is to run the migration with it ON, the way the real pool does.
        let mut conn = pool.acquire().await.unwrap();
        sqlx::query("pragma foreign_keys = on")
            .execute(&mut *conn)
            .await
            .unwrap();

        // The schema as it stood before token hashing, from the files themselves so this can't
        // drift from what a real deployment has on disk.
        for sql in [
            include_str!("../migrations/0001_init.sql"),
            include_str!("../migrations/0002_model.sql"),
            include_str!("../migrations/0003_oidc_users.sql"),
            include_str!("../migrations/0004_retire_the_shared_dev_token.sql"),
            include_str!("../migrations/0005_project_generation.sql"),
        ] {
            sqlx::raw_sql(sql).execute(&mut *conn).await.unwrap();
        }
        // A user, and a project pointing at it — the row that makes dropping `users` illegal.
        sqlx::query(
            "insert into users (name, token, sub) values ('someone', 'plain-secret', 's1')",
        )
        .execute(&mut *conn)
        .await
        .unwrap();
        sqlx::query(
            "insert into projects (user_id, name, svg, model) values (1, 'sketch', '<svg/>', '')",
        )
        .execute(&mut *conn)
        .await
        .unwrap();

        // …and the migration, wrapped in a transaction exactly as sqlx runs it. That wrapping is
        // load-bearing: it is why `pragma foreign_keys = off` is not an option inside a migration.
        sqlx::raw_sql(&format!(
            "begin;\n{}\ncommit;",
            include_str!("../migrations/0006_hash_tokens.sql")
        ))
        .execute(&mut *conn)
        .await
        .expect("0006 applies over existing rows");

        // The project survived, still attached to its user, with nothing dangling.
        let projects: i64 = sqlx::query_scalar("select count(*) from projects where user_id = 1")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
        assert_eq!(projects, 1, "the project kept its owner");
        let violations: i64 =
            sqlx::query_scalar("select count(*) from pragma_foreign_key_check('projects')")
                .fetch_one(&mut *conn)
                .await
                .unwrap();
        assert_eq!(violations, 0, "no dangling reference left behind");

        // The plaintext is destroyed, not merely hidden behind a renamed column.
        let plain: i64 =
            sqlx::query_scalar("select count(*) from users where retired_plaintext_token = ?")
                .bind("plain-secret")
                .fetch_one(&mut *conn)
                .await
                .unwrap();
        assert_eq!(plain, 0, "the stored secret was overwritten");
        drop(conn);
        assert!(
            db::user_by_token(&pool, "plain-secret")
                .await
                .unwrap()
                .is_none(),
            "and it no longer resolves to anyone"
        );

        // The user is still there and can mint a working credential again.
        let user = db::user_by_sub(&pool, "s1").await.unwrap().expect("kept");
        let fresh = db::rotate_token(&pool, user.id).await.unwrap();
        assert_eq!(
            db::user_by_token(&pool, &fresh).await.unwrap().unwrap().id,
            user.id
        );

        pool.close().await;
        let _ = std::fs::remove_file(&path);
    }

    /// Migration 0006 invalidates every stored token rather than converting it, and this is the
    /// half of that decision worth pinning: a credential that was readable in every backup must
    /// stop working, while the row that held it keeps its id — and therefore its projects.
    ///
    /// It subsumes migration 0004, which retired one *published* token. Nothing carried forward
    /// authenticates now, so the old row needs no special case.
    #[tokio::test]
    async fn no_token_survives_the_move_to_hashes() {
        let (pool, path) = test_pool("olddev").await;
        // A row as the pre-hash build left it: identified by `sub`, its token long since stored.
        let before = db::resolve_user(&pool, "sub-old", "old@example.com", "old")
            .await
            .unwrap();
        let known = db::rotate_token(&pool, before.id).await.unwrap();
        assert_eq!(
            db::user_by_token(&pool, &known).await.unwrap().unwrap().id,
            before.id,
            "it works before the tokens are cleared"
        );

        // What the migration does to every row: the hash goes, so nothing matches.
        sqlx::query("update users set token_hash = null, token_hint = ''")
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            db::user_by_token(&pool, &known).await.unwrap().is_none(),
            "a token stored before the move stops working"
        );
        // A NULL hash must not match a *presented* token either — SQL NULL comparison does the
        // right thing here, but silently, so it is asserted rather than assumed.
        assert!(
            db::user_by_token(&pool, "").await.unwrap().is_none(),
            "and an empty presented token matches no cleared row"
        );

        // The user is still there, and rotating gives them a working credential again.
        let after = db::user_by_sub(&pool, "sub-old")
            .await
            .unwrap()
            .expect("the row survived");
        assert_eq!(after.id, before.id, "same row, so same projects");
        let fresh = db::rotate_token(&pool, before.id).await.unwrap();
        assert_eq!(
            db::user_by_token(&pool, &fresh).await.unwrap().unwrap().id,
            before.id
        );
        assert!(
            after.token_hint.is_empty(),
            "nothing is shown for a cleared token"
        );
        assert_eq!(
            db::user_by_sub(&pool, "sub-old")
                .await
                .unwrap()
                .unwrap()
                .token_hint,
            db::token_hint(&fresh),
            "and the hint identifies the new one"
        );

        // Seeding the dev identity still works on top of all that — a dev restarts all day.
        let dev = db::ensure_dev_user(&pool, db::DEV_TOKEN_DEFAULT)
            .await
            .unwrap();
        db::ensure_dev_user(&pool, db::DEV_TOKEN_DEFAULT)
            .await
            .unwrap();
        assert_eq!(
            db::user_by_token(&pool, db::DEV_TOKEN_DEFAULT)
                .await
                .unwrap()
                .expect("the dev token resolves")
                .id,
            dev.id
        );

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
        // They no longer hold the dev token — that is what  proves — and a
        // fresh one still authenticates them, so being displaced did not lock them out. The token
        // itself is unreadable now (only its hash is stored), so this checks the behaviour rather
        // than the stored value.
        let theirs = db::rotate_token(&pool, other.id).await.unwrap();
        assert!(theirs.starts_with("nib_"));
        assert_eq!(
            db::user_by_token(&pool, &theirs).await.unwrap().unwrap().id,
            other.id
        );

        pool.close().await;
        let _ = std::fs::remove_file(&path);
    }
}
