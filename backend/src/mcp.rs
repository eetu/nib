//! The MCP tool surface (Phase C) — nib's editing engine exposed to an LLM over MCP, nested at
//! `/mcp` on the axum server. Token-authed + project-scoped: each call resolves the user from its
//! bearer token and acts on the connection's active project, whose authoritative `Editor` lives in
//! the shared session registry. Edits funnel through `session::apply_ops`, so they persist to
//! SQLite and broadcast to the browser live. The tools are a thin layer over the same `nib-core`
//! op vocabulary the browser editor runs on: the ops ARE the surface.
//!
//! The surface is shaped to *coach* the model (mirroring the sibling `../maquette` tool): a
//! workflow playbook in the server `instructions`, per-tool descriptions that say when NOT to spend
//! an expensive call, terse one-line acks from mutations (never the whole document), a cheap text
//! `get_document` outline for reasoning, and an opt-in `render_document` for when it needs pixels.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::http::request::Parts;
use base64::prelude::{BASE64_STANDARD, Engine as _};
use resvg::{tiny_skia, usvg};
use rmcp::handler::server::tool::{Parameters, ToolRouter};
use rmcp::model::{CallToolResult, Content, ServerCapabilities, ServerInfo};
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

/// The workflow playbook handed to the model on connect — imperative, cost-aware, and it teaches
/// the two habits that make the output a *document* rather than a pile of paths: name + group.
const INSTRUCTIONS: &str = "nib is a direct-manipulation SVG editor. You co-edit a live document \
with a human in the browser — every change you make persists and syncs to their canvas instantly. \
Authenticate with your bearer token (your nib user token).\n\n\
WORKFLOW (follow this to work well and keep token cost low):\n\
1. list_projects, then open_project (or create_project) to make one active. Coordinates are in the \
document's viewBox units. Z-order = list order: the LAST path draws on top, so build back-to-front \
(background first, details last).\n\
2. get_document is a CHEAP TEXT outline of the structure — one line per path (#index, name, bounds, \
fill/stroke). No image. Use it to plan, to measure, and to resync #indices. Prefer it over rendering \
for anything that isn't 'I need to see what it looks like'.\n\
3. Edit with add_shape (rect/ellipse/line/polygon/star by bounding box), apply_op (the full op \
vocabulary), set_style, boolean_op, group, rename. Mutations return a ONE-LINE ack, not the document \
— that's deliberate; call get_document when you need the current #indices.\n\
4. NAME every shape by its role (add_shape's `name`, or rename afterwards) and GROUP related shapes \
(group) so the result is a labeled, editable hierarchy a human can navigate — e.g. a 'face' group of \
'left-eye'/'right-eye'/'mouth' — not an anonymous pile of paths. get_document echoes names back, so \
good naming compounds. This is the difference between a drawing and a mess: always do it.\n\
5. render_document returns a PNG so you can SEE the result and verify it actually reads correctly \
(get_document gives structure; this gives pixels). Images are token-heavy — render at checkpoints, \
not after every edit; pass a small `width` for a quick glance.\n\
6. Structural ops (group, boolean_op, reorder) RENUMBER #indices. Call get_document afterwards before \
you address paths by index again.\n\n\
MULTI-AGENT TIP: this loop splits well — a strong model plans + edits while reading the cheap text \
outline; a cheaper vision pass calls render_document and reports a terse critique, so the expensive \
context never carries images across iterations.";

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

/// Compact number: 2 decimals, trailing zeros/point trimmed. Keeps the text outline small.
fn r(n: f64) -> String {
    let s = format!("{n:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// Count of live (non-deleted) paths.
fn count_paths(s: &ProjectSession) -> usize {
    s.editor
        .doc()
        .map(|d| d.paths.iter().filter(|p| !p.deleted).count())
        .unwrap_or(0)
}

/// The just-appended path's (#index, id) — `add_drawn` pushes to the tail = top of z-order.
fn tail(s: &ProjectSession) -> (usize, String) {
    s.editor
        .doc()
        .map(|d| {
            let i = d.paths.len().saturating_sub(1);
            (i, d.paths.last().map(|p| p.id.clone()).unwrap_or_default())
        })
        .unwrap_or((0, String::new()))
}

/// A cheap TEXT digest of the open document — viewBox + one line per live path (#index, name, rough
/// bounds, fill/stroke, node count). What an LLM needs to plan edits, an order of magnitude cheaper
/// than echoing verbose JSON. Names are echoed back so descriptive naming pays off on the next edit.
fn outline(s: &ProjectSession) -> String {
    let Some(doc) = s.editor.doc() else {
        return "no document open".to_string();
    };
    let vb = &doc.view_box;
    let mut lines = vec![format!(
        "project {} · viewBox {} {} {} {} · {} paths (address by #index; z-order = list order, last on top)",
        s.project_id,
        r(vb.min_x),
        r(vb.min_y),
        r(vb.width),
        r(vb.height),
        count_paths(s),
    )];
    for (i, p) in doc.paths.iter().enumerate() {
        if p.deleted {
            continue;
        }
        let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for sp in &p.subpaths {
            for n in &sp.nodes {
                minx = minx.min(n.point.x);
                miny = miny.min(n.point.y);
                maxx = maxx.max(n.point.x);
                maxy = maxy.max(n.point.y);
            }
        }
        let bbox = if minx <= maxx {
            format!(
                "[{} {} {}×{}]",
                r(minx),
                r(miny),
                r(maxx - minx),
                r(maxy - miny)
            )
        } else {
            "[empty]".to_string()
        };
        let style = |k: &str| {
            p.style_override
                .as_ref()
                .and_then(|s| s.get(k))
                .or_else(|| p.attributes.as_ref().and_then(|a| a.get(k)))
                .cloned()
        };
        let id = if p.id.is_empty() { "-" } else { p.id.as_str() };
        let fill = style("fill").unwrap_or_else(|| "none".to_string());
        let stroke = style("stroke")
            .map(|s| format!(" stroke {s}"))
            .unwrap_or_default();
        let nodes: usize = p.subpaths.iter().map(|sp| sp.nodes.len()).sum();
        let hidden = if p.hidden { " hidden" } else { "" };
        lines.push(format!(
            "#{i} {id} {bbox} fill {fill}{stroke} {nodes}n{hidden}"
        ));
    }
    lines.join("\n")
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

/// Rasterize an SVG string to PNG bytes with resvg, scaled so its longest side is ~`target` px
/// and composited on white (a preview surface — nib's canvas backdrop is orthogonal). Pure-Rust,
/// in-process; text without embedded/system fonts won't render (this is a path editor's preview).
fn render_png(svg: &str, target: f32) -> Result<Vec<u8>, String> {
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_str(svg, &opt).map_err(|e| e.to_string())?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    if w <= 0.0 || h <= 0.0 {
        return Err("document has no drawable area".into());
    }
    let scale = target / w.max(h);
    let pw = (w * scale).round().max(1.0) as u32;
    let ph = (h * scale).round().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(pw, ph).ok_or("pixmap allocation failed")?;
    pixmap.fill(tiny_skia::Color::WHITE);
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().map_err(|e| e.to_string())
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
    /// A descriptive id/name for the shape, e.g. "left-eye" or "caesar-toga". Strongly recommended —
    /// it becomes the path's id, shows in the outline + the human's layers panel, and is how you
    /// (and they) recognise it later. Omit only for throwaway scaffolding.
    #[serde(default)]
    pub name: Option<String>,
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

#[derive(Deserialize, schemars::JsonSchema)]
pub struct GroupParams {
    /// The path #indices to group (2+). They must share one parent — top-level paths always do.
    pub indices: Vec<usize>,
    /// A descriptive name for the group, e.g. "face" or "die-1". Becomes the `<g>` id.
    pub name: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct RenameParams {
    /// The path's #index (from get_document).
    pub index: usize,
    /// The new descriptive name/id, e.g. "caesar-toga".
    pub name: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct RenderParams {
    /// Target longest-side in px (default 512, clamped 128–1024). Smaller = cheaper (fewer tokens).
    #[serde(default)]
    pub width: Option<f64>,
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
        description = "Create a new blank project and make it active. Returns its id. Then build back-to-front (background first), naming + grouping shapes as you go."
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
        description = "Open one of your projects and make it active. Returns the cheap text outline of its paths + viewBox."
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
        Ok(outline(&sess.lock().unwrap()))
    }

    #[tool(
        description = "Cheap TEXT outline of the active project — viewBox + one line per path (#index, name, bounds, fill/stroke, node count). No image. Use it to plan, measure, and resync #indices after structural ops; prefer it over render_document for reasoning about structure."
    )]
    async fn get_document(&self, ctx: RequestContext<RoleServer>) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        Ok(outline(&sess.lock().unwrap()))
    }

    #[tool(
        description = "Get the active project's document as raw SVG markup. Verbose — prefer get_document to reason about structure; use this only when you need the exact source."
    )]
    async fn get_svg(&self, ctx: RequestContext<RoleServer>) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let svg = sess.lock().unwrap().editor.to_svg();
        Ok(svg)
    }

    #[tool(
        description = "Render the active project to a PNG and return it as an image, so you can SEE the drawing and verify it reads correctly (get_document gives structure; this gives pixels). Image-heavy — render at checkpoints, not after every edit; pass a smaller `width` for a cheap glance. Composited on white; text needs fonts and may not render."
    )]
    async fn render_document(
        &self,
        Parameters(p): Parameters<RenderParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let svg = sess.lock().unwrap().editor.to_svg();
        let target = p.width.unwrap_or(512.0).clamp(128.0, 1024.0) as f32;
        let png = render_png(&svg, target).map_err(|e| bad(format!("render failed: {e}")))?;
        let b64 = BASE64_STANDARD.encode(&png);
        Ok(CallToolResult::success(vec![Content::image(
            b64,
            "image/png".to_string(),
        )]))
    }

    #[tool(
        description = "Apply one editing operation to the active project — the full nib op vocabulary. `op` is a JSON object tagged by `type`. Examples: {\"type\":\"movePathBy\",\"path\":0,\"dx\":10,\"dy\":0}; {\"type\":\"setStyle\",\"path\":0,\"key\":\"fill\",\"value\":\"#ff0000\"}; {\"type\":\"deletePath\",\"path\":2}; {\"type\":\"booleanOp\",\"op\":\"union\",\"paths\":[0,1],\"id\":\"u1\"}. Returns a one-line ack, not the document. Structural ops (groupNodes/booleanOp/reorder*) renumber #indices — call get_document after. Persists + broadcasts to any live browser."
    )]
    async fn apply_op(
        &self,
        Parameters(p): Parameters<ApplyOpParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let n = session::apply_ops(&sess, &self.pool, vec![p.op.clone()], "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad("the op did not apply (missing target / no-op)"));
        }
        let t = p.op.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let s = sess.lock().unwrap();
        let ack = match t {
            "addPath" | "addShape" => {
                let (idx, id) = tail(&s);
                format!("added #{idx} \"{id}\" · {} paths", count_paths(&s))
            }
            "groupNodes" | "ungroupNode" | "reorderNode" | "moveTreeNode" | "reorderPath"
            | "booleanOp" | "combinePaths" | "releaseCompound" => {
                format!("applied {t} · #indices renumbered, call get_document")
            }
            _ => format!("applied {t} · {} paths", count_paths(&s)),
        };
        Ok(ack)
    }

    #[tool(
        description = "Add a shape by bounding box. shape ∈ {ellipse, rect, line, polygon, star}; x/y/w/h in viewBox units; optional fill/stroke colours. Pass `name` to give it a descriptive id (do this — see the workflow). Returns the new path's #index."
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
        let id = p
            .name
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.gen_id(&p.shape));
        let op = json!({ "type": "addShape", "id": id, "spec": spec, "attributes": attrs });
        session::apply_ops(&sess, &self.pool, vec![op], "mcp").map_err(bad)?;
        let s = sess.lock().unwrap();
        let (idx, rid) = tail(&s);
        Ok(format!(
            "added #{idx} \"{rid}\" · {} paths",
            count_paths(&s)
        ))
    }

    #[tool(
        description = "Set paint/stroke on a path (by #index): any of fill, stroke, strokeWidth, opacity."
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
            return Err(bad("no style applied (bad #index?)"));
        }
        Ok(format!("styled #{}", p.index))
    }

    #[tool(
        description = "Combine paths with a boolean op (union/subtract/intersect/exclude) by their #indices (2+). The inputs are replaced by one result path. Renumbers #indices — call get_document after."
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
        Ok(format!(
            "boolean {} on {} paths → #indices renumbered, call get_document",
            p.op,
            p.indices.len()
        ))
    }

    #[tool(
        description = "Group related paths (by their #indices, 2+) into a named <g> so the document is a labeled hierarchy a human can navigate. The paths must share one parent — top-level paths always do. Renumbers #indices — call get_document after."
    )]
    async fn group(
        &self,
        Parameters(p): Parameters<GroupParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        // Map #indices → tree uids under the lock (no await held), then apply.
        let uids: Vec<String> = {
            let s = sess.lock().unwrap();
            let doc = s.editor.doc().ok_or_else(|| bad("no document"))?;
            let mut v = Vec::with_capacity(p.indices.len());
            for &i in &p.indices {
                let pe = doc
                    .paths
                    .get(i)
                    .ok_or_else(|| bad(format!("no path at #{i}")))?;
                if pe.deleted {
                    return Err(bad(format!("path #{i} is deleted")));
                }
                if pe.uid.is_empty() {
                    return Err(bad(format!("path #{i} has no tree uid (can't group)")));
                }
                v.push(pe.uid.clone());
            }
            v
        };
        if uids.len() < 2 {
            return Err(bad("group needs at least 2 path indices"));
        }
        let k = uids.len();
        let op = json!({ "type": "groupNodes", "uids": uids, "uid": self.gen_id("grp"), "name": p.name });
        let n = session::apply_ops(&sess, &self.pool, vec![op], "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad(
                "group did not apply — the paths must share one parent (top-level paths do)",
            ));
        }
        Ok(format!(
            "grouped {k} paths as \"{}\" → #indices renumbered, call get_document",
            p.name
        ))
    }

    #[tool(
        description = "Give a path (by #index) a descriptive, human-readable name/id, e.g. \"caesar-toga\". Do this for shapes you didn't already name at creation — it shows in the outline + the human's layers panel."
    )]
    async fn rename(
        &self,
        Parameters(p): Parameters<RenameParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let op = json!({ "type": "renamePath", "path": p.index, "name": p.name });
        let n = session::apply_ops(&sess, &self.pool, vec![op], "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad("rename failed (bad #index or empty name)"));
        }
        Ok(format!("renamed #{} → \"{}\"", p.index, p.name))
    }
}

#[tool_handler]
impl ServerHandler for NibMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(INSTRUCTIONS.to_string()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::render_png;

    #[test]
    fn renders_svg_to_png() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100" fill="#3f86d4"/><circle cx="50" cy="50" r="30" fill="#ffd85e"/></svg>"##;
        let png = render_png(svg, 640.0).expect("render");
        assert_eq!(&png[..4], b"\x89PNG", "PNG magic bytes");
        let pm = resvg::tiny_skia::Pixmap::decode_png(&png).expect("decode");
        // viewBox 100×100 scaled so the longest side is 640.
        assert_eq!((pm.width(), pm.height()), (640, 640));
        // Centre pixel is inside the drawn shapes, so it must not be the white backdrop.
        let idx = ((pm.height() / 2 * pm.width() + pm.width() / 2) * 4) as usize;
        assert_ne!(
            &pm.data()[idx..idx + 3],
            &[255, 255, 255],
            "something was drawn"
        );
    }
}
