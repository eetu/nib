//! The MCP tool surface (Phase C) — nib's editing engine exposed to an LLM over MCP, nested at
//! `/mcp` on the axum server. Token-authed + project-scoped: each call resolves the user from its
//! bearer token and acts on the connection's active project, whose authoritative `Editor` lives in
//! the shared session registry. Edits funnel through `session::apply_ops`, so they persist to
//! SQLite and broadcast to the browser live. The tools are a thin layer over the same `nib-core`
//! op vocabulary the browser editor runs on: the ops ARE the surface.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::http::request::Parts;
use rmcp::handler::server::tool::{Parameters, ToolRouter};
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::json;
use sqlx::SqlitePool;

use crate::db::{self, User};
use crate::session::{self, ProjectSession, Sessions};
use crate::{AppState, auth};

const BLANK_SVG: &str =
    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\">\n</svg>";

#[derive(Clone)]
pub struct NibMcp {
    pool: SqlitePool,
    sessions: Sessions,
    /// The project this connection is editing (set by `open_project`/`create_project`).
    active: Arc<std::sync::Mutex<Option<i64>>>,
    /// Monotonic suffix for generated element ids.
    next_id: Arc<AtomicU64>,
    tool_router: ToolRouter<NibMcp>,
}

impl NibMcp {
    pub fn new(state: &AppState) -> Self {
        NibMcp {
            pool: state.pool.clone(),
            sessions: state.sessions.clone(),
            active: Arc::new(std::sync::Mutex::new(None)),
            next_id: Arc::new(AtomicU64::new(1)),
            tool_router: Self::tool_router(),
        }
    }

    /// Resolve the authenticated user from the request's bearer token.
    async fn user(&self, ctx: &RequestContext<RoleServer>) -> Result<User, ErrorData> {
        let parts = ctx
            .extensions
            .get::<Parts>()
            .ok_or_else(|| bad("no request context"))?;
        auth::user_from_parts(&self.pool, parts)
            .await
            .ok_or_else(|| bad("unauthorized — set Authorization: Bearer <token>"))
    }

    /// The active project's session (the one `open_project`/`create_project` selected).
    async fn active_session(
        &self,
        user: &User,
    ) -> Result<Arc<std::sync::Mutex<ProjectSession>>, ErrorData> {
        let id =
            self.active.lock().unwrap().ok_or_else(|| {
                bad("no project open — call open_project or create_project first")
            })?;
        session::open(&self.pool, &self.sessions, user.id, id)
            .await
            .map_err(bad)
    }

    fn gen_id(&self, prefix: &str) -> String {
        format!("{prefix}-{}", self.next_id.fetch_add(1, Ordering::Relaxed))
    }
}

fn bad(msg: impl Into<String>) -> ErrorData {
    ErrorData::invalid_params(msg.into(), None)
}

/// A compact structured view of the open document — viewBox + each editable path by integer
/// `index` (id, rough bounds, fill/stroke, node count). What an LLM needs to plan edits.
fn summary(s: &ProjectSession) -> serde_json::Value {
    let Some(doc) = s.editor.doc() else {
        return json!({ "open": false });
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
                json!({ "x": minx, "y": miny, "w": maxx - minx, "h": maxy - miny })
            } else {
                serde_json::Value::Null
            };
            json!({
                "index": i, "id": p.id, "added": p.added, "hidden": p.hidden,
                "nodes": p.subpaths.iter().map(|sp| sp.nodes.len()).sum::<usize>(),
                "bounds": bounds, "fill": style("fill"), "stroke": style("stroke"),
            })
        })
        .collect();
    json!({
        "project": s.project_id,
        "viewBox": { "minX": vb.min_x, "minY": vb.min_y, "width": vb.width, "height": vb.height },
        "paths": paths,
    })
}

/// Bounding-box → a `ShapeSpec` JSON for `add_shape`. `None` for an unknown shape.
fn shape_spec(shape: &str, x: f64, y: f64, w: f64, h: f64) -> Option<serde_json::Value> {
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let r = w.min(h) / 2.0;
    let up = -std::f64::consts::FRAC_PI_2;
    Some(match shape {
        "ellipse" | "circle" => json!({"shape":"ellipse","cx":cx,"cy":cy,"rx":w/2.0,"ry":h/2.0}),
        "rect" => json!({"shape":"rect","x0":x,"y0":y,"x1":x+w,"y1":y+h}),
        "line" => json!({"shape":"line","x0":x,"y0":y,"x1":x+w,"y1":y+h}),
        "polygon" => json!({"shape":"polygon","cx":cx,"cy":cy,"r":r,"sides":6,"rotation":up}),
        "star" => {
            json!({"shape":"star","cx":cx,"cy":cy,"outer":r,"inner":r*0.5,"points":5,"rotation":up})
        }
        _ => return None,
    })
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CreateParams {
    /// A name for the new project.
    pub name: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct OpenParams {
    /// The project id to open (from `list_projects`).
    pub id: i64,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ApplyOpParams {
    /// The operation as a JSON object tagged by `type` (the nib op vocabulary).
    pub op: serde_json::Value,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct AddShapeParams {
    /// One of: ellipse, rect, line, polygon, star.
    pub shape: String,
    /// Bounding box of the shape, in viewBox units.
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(default)]
    pub fill: Option<String>,
    #[serde(default)]
    pub stroke: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SetStyleParams {
    /// The path's integer index (from get_document).
    pub index: usize,
    #[serde(default)]
    pub fill: Option<String>,
    #[serde(default)]
    pub stroke: Option<String>,
    #[serde(default, rename = "strokeWidth")]
    pub stroke_width: Option<f64>,
    #[serde(default)]
    pub opacity: Option<f64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct BooleanParams {
    /// One of: union, subtract, intersect, exclude.
    pub op: String,
    /// The path indices to combine (2+).
    pub indices: Vec<usize>,
}

#[tool_router]
impl NibMcp {
    #[tool(description = "List your projects (id, name, updated_at).")]
    async fn list_projects(&self, ctx: RequestContext<RoleServer>) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let projects = db::list_projects(&self.pool, user.id)
            .await
            .map_err(|e| bad(e.to_string()))?;
        Ok(serde_json::to_string(&projects).unwrap_or_default())
    }

    #[tool(
        description = "Create a new (blank) project and make it the active one. Returns its id."
    )]
    async fn create_project(
        &self,
        Parameters(p): Parameters<CreateParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let id = db::create_project(&self.pool, user.id, &p.name, BLANK_SVG)
            .await
            .map_err(|e| bad(e.to_string()))?;
        *self.active.lock().unwrap() = Some(id);
        Ok(json!({ "id": id, "name": p.name }).to_string())
    }

    #[tool(
        description = "Open one of your projects into the editing session and make it active. Returns a structured summary of its paths + viewBox."
    )]
    async fn open_project(
        &self,
        Parameters(p): Parameters<OpenParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = session::open(&self.pool, &self.sessions, user.id, p.id)
            .await
            .map_err(bad)?;
        *self.active.lock().unwrap() = Some(p.id);
        let out = serde_json::to_string(&summary(&sess.lock().unwrap())).unwrap_or_default();
        Ok(out)
    }

    #[tool(
        description = "Summarize the active project: viewBox + every editable path (integer `index`, id, rough bounds, fill, stroke, node count). Address paths by `index` in ops."
    )]
    async fn get_document(&self, ctx: RequestContext<RoleServer>) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let out = serde_json::to_string(&summary(&sess.lock().unwrap())).unwrap_or_default();
        Ok(out)
    }

    #[tool(description = "Get the active project's document serialized to SVG markup.")]
    async fn get_svg(&self, ctx: RequestContext<RoleServer>) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let svg = sess.lock().unwrap().editor.to_svg();
        Ok(svg)
    }

    #[tool(
        description = "Apply one editing operation to the active project — the full nib op vocabulary. `op` is a JSON object tagged by `type`. Examples: {\"type\":\"movePathBy\",\"path\":0,\"dx\":10,\"dy\":0}; {\"type\":\"setStyle\",\"path\":0,\"key\":\"fill\",\"value\":\"#ff0000\"}; {\"type\":\"deletePath\",\"path\":2}; {\"type\":\"booleanOp\",\"op\":\"union\",\"paths\":[0,1],\"id\":\"u1\"}. Persists + broadcasts to any live browser editing the same project."
    )]
    async fn apply_op(
        &self,
        Parameters(p): Parameters<ApplyOpParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let n = session::apply_ops(&sess, &self.pool, vec![p.op], "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad("the op did not apply (missing target / no-op)"));
        }
        let out = serde_json::to_string(&summary(&sess.lock().unwrap())).unwrap_or_default();
        Ok(out)
    }

    #[tool(
        description = "Add a shape to the active project by bounding box. shape ∈ {ellipse, rect, line, polygon, star}; x/y/w/h in viewBox units; optional fill/stroke colours."
    )]
    async fn add_shape(
        &self,
        Parameters(p): Parameters<AddShapeParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let spec = shape_spec(&p.shape, p.x, p.y, p.w, p.h)
            .ok_or_else(|| bad(format!("unknown shape: {}", p.shape)))?;
        let mut attrs = serde_json::Map::new();
        if let Some(f) = &p.fill {
            attrs.insert("fill".into(), json!(f));
        }
        if let Some(s) = &p.stroke {
            attrs.insert("stroke".into(), json!(s));
        }
        let op = json!({ "type": "addShape", "id": self.gen_id(&p.shape), "spec": spec, "attributes": attrs });
        session::apply_ops(&sess, &self.pool, vec![op], "mcp").map_err(bad)?;
        let out = serde_json::to_string(&summary(&sess.lock().unwrap())).unwrap_or_default();
        Ok(out)
    }

    #[tool(
        description = "Set paint/stroke on a path (by integer index): any of fill, stroke, strokeWidth, opacity."
    )]
    async fn set_style(
        &self,
        Parameters(p): Parameters<SetStyleParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let mut ops = Vec::new();
        let mut push = |key: &str, val: String| {
            ops.push(json!({ "type": "setStyle", "path": p.index, "key": key, "value": val }));
        };
        if let Some(f) = &p.fill {
            push("fill", f.clone());
        }
        if let Some(s) = &p.stroke {
            push("stroke", s.clone());
        }
        if let Some(w) = p.stroke_width {
            push("stroke-width", w.to_string());
        }
        if let Some(o) = p.opacity {
            push("opacity", o.to_string());
        }
        if ops.is_empty() {
            return Err(bad(
                "nothing to set — pass at least one of fill/stroke/strokeWidth/opacity",
            ));
        }
        let n = session::apply_ops(&sess, &self.pool, ops, "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad("no style applied (bad index?)"));
        }
        let out = serde_json::to_string(&summary(&sess.lock().unwrap())).unwrap_or_default();
        Ok(out)
    }

    #[tool(
        description = "Combine paths with a boolean op (union/subtract/intersect/exclude) by their integer indices (2+). The inputs are replaced by one result path."
    )]
    async fn boolean_op(
        &self,
        Parameters(p): Parameters<BooleanParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        if p.indices.len() < 2 {
            return Err(bad("boolean_op needs at least 2 path indices"));
        }
        let op = json!({ "type": "booleanOp", "op": p.op, "paths": p.indices, "id": self.gen_id(&p.op) });
        let n = session::apply_ops(&sess, &self.pool, vec![op], "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad(
                "boolean op did not apply (check the op name + indices)",
            ));
        }
        let out = serde_json::to_string(&summary(&sess.lock().unwrap())).unwrap_or_default();
        Ok(out)
    }
}

#[tool_handler]
impl ServerHandler for NibMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "nib is an SVG path editor. Authenticate with a bearer token (your nib user token). \
                 Workflow: list_projects → open_project (or create_project) → get_document (paths are \
                 addressed by integer `index`) → edit with apply_op / add_shape / set_style / \
                 boolean_op → changes persist + sync to any live browser editing the same project. \
                 Coordinates are in the document's viewBox units."
                    .to_string(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
