//! nib-backend (Phase C1) — a rust-axum server that links `nib-core` **natively** (the same
//! engine the browser drives via WASM), serves the built SPA, and owns a folder of real
//! `.svg` files. Writes are validated through the core's parser, so broken markup never lands
//! on disk. Op-log-over-WebSocket sync (C2) + the MCP tool surface (C3) build on this.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use serde::Serialize;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

mod mcp;

#[derive(Clone)]
struct AppState {
    /// The directory of `.svg` documents the server owns.
    docs: Arc<PathBuf>,
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

/// Reject anything that isn't a bare `*.svg` filename (no path traversal).
pub(crate) fn safe_name(name: &str) -> Option<&str> {
    let ok = !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
        && name.to_ascii_lowercase().ends_with(".svg");
    ok.then_some(name)
}

async fn list_files(State(st): State<AppState>) -> Json<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(st.docs.as_ref())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.to_ascii_lowercase().ends_with(".svg"))
        .collect();
    names.sort();
    Json(names)
}

async fn read_file(
    Path(name): Path<String>,
    State(st): State<AppState>,
) -> Result<String, StatusCode> {
    let name = safe_name(&name).ok_or(StatusCode::BAD_REQUEST)?;
    std::fs::read_to_string(st.docs.join(name)).map_err(|_| StatusCode::NOT_FOUND)
}

async fn write_file(
    Path(name): Path<String>,
    State(st): State<AppState>,
    body: String,
) -> Result<StatusCode, (StatusCode, String)> {
    let name = safe_name(&name).ok_or((StatusCode::BAD_REQUEST, "invalid filename".to_string()))?;
    // Validate through the core parser so we never persist markup the editor can't reopen.
    nib_core::model::document::parse_svg(&body).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    std::fs::create_dir_all(st.docs.as_ref()).ok();
    std::fs::write(st.docs.join(name), body)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

#[tokio::main]
async fn main() {
    let docs = PathBuf::from(std::env::var("NIB_DOCS").unwrap_or_else(|_| "docs".to_string()));
    let dist =
        PathBuf::from(std::env::var("NIB_DIST").unwrap_or_else(|_| "../frontend/dist".to_string()));
    std::fs::create_dir_all(&docs).ok();

    let state = AppState {
        docs: Arc::new(docs.clone()),
    };

    // Serve the SPA, falling back to index.html for client-side deep links (family contract).
    let spa = ServeDir::new(&dist).fallback(ServeFile::new(dist.join("index.html")));

    // The MCP tool surface (C3), nested at /mcp via the Streamable-HTTP transport. It shares one
    // editing session with the process, so an LLM and (later, C2) the live UI drive the same doc.
    let mcp_session = Arc::new(Mutex::new(mcp::Session::new()));
    let mcp_docs = Arc::new(docs.clone());
    let mcp_service = StreamableHttpService::new(
        move || Ok(mcp::NibMcp::new(mcp_session.clone(), mcp_docs.clone())),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    );

    let app = Router::new()
        .route("/api/version", get(version))
        .route("/api/files", get(list_files))
        .route("/api/files/{name}", get(read_file).put(write_file))
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
        "nib-backend on http://{addr}  (docs: {}, dist: {})",
        docs.display(),
        dist.display()
    );
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind listener");
    axum::serve(listener, app).await.expect("serve");
}
