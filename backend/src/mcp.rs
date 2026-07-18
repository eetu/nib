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

use nib_core::model::tree::RenderNode;
use nib_core::model::types::SvgDocument;

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
vocabulary), set_style, boolean_op, group, rename, rotate (degrees clockwise about a shape's centre). \
Mutations return a ONE-LINE ack, not the document — that's deliberate; call get_document when you \
need the current #indices.\n\
4. NAME every shape by its role (add_shape's `name`, or rename afterwards) and GROUP related shapes \
(group) so the result is a labeled, editable hierarchy a human can navigate — e.g. a 'face' group of \
'left-eye'/'right-eye'/'mouth' — not an anonymous pile of paths. get_document echoes names back, so \
good naming compounds. This is the difference between a drawing and a mess: always do it. To act on \
what your co-author means ('make the hand bigger'), call `find` to resolve the name to object(s); if \
more than one matches (two 'hand's), DON'T guess — show them the candidates and ask which (left or \
right?). Names are the shared vocabulary; the #index/uid are how you then address the one they meant. \
For a REPEATED compound (dice, windows, tree leaves, icons), `create_component` from its shapes once, \
then `stamp` instances instead of re-drawing them — far fewer ops, and editing the definition updates \
every instance (`list_components` shows what's defined).\n\
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
    // Components + which paths are definition parts (so def-space bounds don't confuse the LLM).
    let (comps, part_comp) = component_info(doc);
    if !comps.is_empty() {
        let names: Vec<String> = comps
            .iter()
            .map(|c| {
                format!(
                    "{} ({} parts, {}×)",
                    c["name"].as_str().unwrap_or("?"),
                    c["parts"],
                    c["instances"]
                )
            })
            .collect();
        lines.push(format!(
            "components: {} — a `<use>` renders one; edit a part to update all",
            names.join(", ")
        ));
    }
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
        let in_comp = part_comp
            .get(&p.uid)
            .map(|n| format!(" [in component: {n}]"))
            .unwrap_or_default();
        lines.push(format!(
            "#{i} {id} {bbox} fill {fill}{stroke} {nodes}n{hidden}{in_comp}"
        ));
    }
    lines.join("\n")
}

/// Resolve a human name to the paths that match it — exact name first, then partial (contains),
/// case-insensitive. Each candidate carries its `#index`, `name`, `uid`, and rough `bounds` so an
/// LLM can disambiguate ("two 'hand's — left or right?"). The name-addressing cornerstone: a
/// co-author refers by name, this maps to the object(s) they mean.
fn find_by_name(doc: &SvgDocument, query: &str) -> Vec<serde_json::Value> {
    let q = query.trim().to_lowercase();
    let mut matches: Vec<serde_json::Value> = Vec::new();
    for (i, path) in doc.paths.iter().enumerate() {
        if path.deleted {
            continue;
        }
        let name = path.id.to_lowercase();
        let exact = name == q;
        if !exact && !name.contains(&q) {
            continue;
        }
        let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for sp in &path.subpaths {
            for n in &sp.nodes {
                minx = minx.min(n.point.x);
                miny = miny.min(n.point.y);
                maxx = maxx.max(n.point.x);
                maxy = maxy.max(n.point.y);
            }
        }
        let bounds = if minx <= maxx {
            json!({ "x": r(minx), "y": r(miny), "w": r(maxx - minx), "h": r(maxy - miny) })
        } else {
            serde_json::Value::Null
        };
        matches.push(json!({
            "index": i, "name": path.id, "uid": path.uid, "bounds": bounds, "exact": exact,
        }));
    }
    // Exact-name matches first, so a precise reference ranks above partial hits.
    matches.sort_by_key(|m| !m.get("exact").and_then(|e| e.as_bool()).unwrap_or(false));
    matches
}

/// The document's components (a `<g id>` directly inside a `<defs>`) as summary JSON, plus a map of
/// each definition part's `uid` → its component name (so the outline can label def-paths).
fn component_info(
    doc: &SvgDocument,
) -> (
    Vec<serde_json::Value>,
    std::collections::HashMap<String, String>,
) {
    let mut summaries = Vec::new();
    let mut part_comp = std::collections::HashMap::new();
    let Some(tree) = doc.tree.as_ref() else {
        return (summaries, part_comp);
    };
    let roots = tree.render_children();
    let mut uses: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    count_uses(&roots, &mut uses);
    collect_components(&roots, &uses, &mut summaries, &mut part_comp);
    (summaries, part_comp)
}

fn count_uses(nodes: &[RenderNode], uses: &mut std::collections::HashMap<String, usize>) {
    for n in nodes {
        if let RenderNode::Element {
            tag,
            attrs,
            children,
            ..
        } = n
        {
            if tag == "use" {
                if let Some(h) = attrs.get("href").or_else(|| attrs.get("xlink:href")) {
                    *uses
                        .entry(h.trim_start_matches('#').to_string())
                        .or_insert(0) += 1;
                }
            }
            count_uses(children, uses);
        }
    }
}

fn collect_part_uids(node: &RenderNode, out: &mut Vec<String>) {
    if let RenderNode::Element { children, .. } = node {
        for c in children {
            if let RenderNode::Element { uid, .. } = c {
                out.push(uid.clone());
                collect_part_uids(c, out);
            }
        }
    }
}

fn collect_components(
    nodes: &[RenderNode],
    uses: &std::collections::HashMap<String, usize>,
    summaries: &mut Vec<serde_json::Value>,
    part_comp: &mut std::collections::HashMap<String, String>,
) {
    for n in nodes {
        let RenderNode::Element { tag, children, .. } = n else {
            continue;
        };
        if tag == "defs" {
            for c in children {
                if let RenderNode::Element {
                    tag: ct,
                    attrs,
                    uid,
                    ..
                } = c
                {
                    if ct == "g" {
                        if let Some(id) = attrs.get("id") {
                            let mut parts = Vec::new();
                            collect_part_uids(c, &mut parts);
                            for p in &parts {
                                part_comp.insert(p.clone(), id.clone());
                            }
                            summaries.push(json!({
                                "name": id, "uid": uid, "parts": parts.len(),
                                "instances": uses.get(id).copied().unwrap_or(0),
                            }));
                        }
                    }
                }
            }
        } else {
            collect_components(children, uses, summaries, part_comp);
        }
    }
}

/// Every tree-node uid whose `id` equals `name` — resolves a co-author's name to the node to act
/// on, matching BOTH shapes and `<g>` groups (groups carry an `id`), so a caller can nest existing
/// groups. Ambiguous (>1) is surfaced to the caller rather than guessed.
fn uids_by_id(doc: &SvgDocument, name: &str) -> Vec<String> {
    fn walk(nodes: &[RenderNode], name: &str, out: &mut Vec<String>) {
        for n in nodes {
            if let RenderNode::Element {
                attrs,
                uid,
                children,
                ..
            } = n
            {
                if attrs.get("id").map(String::as_str) == Some(name) {
                    out.push(uid.clone());
                }
                walk(children, name, out);
            }
        }
    }
    let mut out = Vec::new();
    if let Some(tree) = doc.tree.as_ref() {
        walk(&tree.render_children(), name, &mut out);
    }
    out
}

/// Bounding-box → a `ShapeSpec` JSON for `add_shape`. `radius` rounds a rect's corners (ignored by
/// other shapes). `None` for an unknown shape.
fn shape_spec(
    shape: &str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    radius: f64,
) -> Option<serde_json::Value> {
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let r = w.min(h) / 2.0;
    let up = -std::f64::consts::FRAC_PI_2;
    Some(match shape {
        "ellipse" | "circle" => json!({"shape":"ellipse","cx":cx,"cy":cy,"rx":w/2.0,"ry":h/2.0}),
        "rect" => json!({"shape":"rect","x0":x,"y0":y,"x1":x+w,"y1":y+h,"rx":radius,"ry":radius}),
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
    /// Corner radius (viewBox units) for a `rect` — rounds its corners. Ignored by other shapes.
    #[serde(default)]
    pub radius: Option<f64>,
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
pub struct GroupNamedParams {
    /// Names/ids of shapes and/or existing groups to wrap (2+). They must share one parent.
    pub names: Vec<String>,
    /// A descriptive name for the new parent group, e.g. "caesar".
    pub name: String,
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
pub struct CreateComponentParams {
    /// The path #indices to turn into a reusable component (1+). They must share one parent.
    pub indices: Vec<usize>,
    /// A unique name for the component (its `<g>` id + the `<use href>` target), e.g. "die".
    pub name: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct StampParams {
    /// The component name to stamp (from create_component / list_components).
    pub component: String,
    /// Optional placement offset (viewBox units) so the instance doesn't overlap the others.
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct RotateParams {
    /// The path's #index (from get_document).
    pub index: usize,
    /// Rotation in degrees, clockwise (like SVG `rotate()`).
    pub degrees: f64,
    /// Optional pivot (viewBox units). Defaults to the shape's own bounding-box centre; pass a
    /// shared pivot to rotate several shapes as one rigid group.
    #[serde(default)]
    pub cx: Option<f64>,
    #[serde(default)]
    pub cy: Option<f64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct FlipParams {
    /// The path's #index (from get_document).
    pub index: usize,
    /// "horizontal" (left↔right) or "vertical" (top↕bottom).
    pub axis: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReorderParams {
    /// The path's #index (from get_document).
    pub index: usize,
    /// front | back | forward | backward.
    pub r#where: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DropShadowParams {
    /// The path's #index (from get_document).
    pub index: usize,
    /// Shadow offset (viewBox units). Default 2, 2.
    #[serde(default)]
    pub dx: Option<f64>,
    #[serde(default)]
    pub dy: Option<f64>,
    /// Blur amount (feGaussianBlur stdDeviation). Default 2.
    #[serde(default)]
    pub blur: Option<f64>,
    /// Shadow colour + opacity. Default black at 0.4.
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub opacity: Option<f64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct RenderParams {
    /// Target longest-side in px (default 512, clamped 128–1024). Smaller = cheaper (fewer tokens).
    #[serde(default)]
    pub width: Option<f64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct FindParams {
    /// A human name/description to resolve, e.g. "hand" or "left-eye". Case-insensitive; matches a
    /// path whose name equals or contains it.
    pub name: String,
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
        description = "Resolve a human name/description (what your co-author calls something) to the object(s) that match it. Returns candidates, each with #index, name, bounds, and uid. If 0 match, nothing has that name; if >1 (e.g. two 'hand's), DON'T guess — show the candidates and ask the human which (left or right?), then act on the one they mean by its #index. Case-insensitive; matches exact names first, then partial."
    )]
    async fn find(
        &self,
        Parameters(p): Parameters<FindParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        if p.name.trim().is_empty() {
            return Err(bad("find needs a non-empty name"));
        }
        let s = sess.lock().unwrap();
        let doc = s.editor.doc().ok_or_else(|| bad("no document"))?;
        let matches = find_by_name(doc, &p.name);
        Ok(json!({ "query": p.name, "count": matches.len(), "matches": matches }).to_string())
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
        let spec = shape_spec(&p.shape, p.x, p.y, p.w, p.h, p.radius.unwrap_or(0.0))
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
        description = "Group shapes AND/OR existing groups into a new named parent group, addressed by name/id — the way to NEST groups (e.g. group \"robe\",\"head\",\"arm\" → \"caesar\"). `group` takes path #indices only and can't reach a group; this can. Names must be unambiguous and share one parent. Renumbers #indices — call get_document after."
    )]
    async fn group_named(
        &self,
        Parameters(p): Parameters<GroupNamedParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        // Resolve each name → a single tree uid (shape or group) under the lock, then apply.
        let uids: Vec<String> = {
            let s = sess.lock().unwrap();
            let doc = s.editor.doc().ok_or_else(|| bad("no document"))?;
            let mut v = Vec::with_capacity(p.names.len());
            for nm in &p.names {
                let m = uids_by_id(doc, nm);
                match m.len() {
                    0 => return Err(bad(format!("no shape or group named \"{nm}\""))),
                    1 => v.push(m.into_iter().next().unwrap()),
                    k => {
                        return Err(bad(format!(
                            "\"{nm}\" is ambiguous ({k} matches) — rename to disambiguate"
                        )));
                    }
                }
            }
            v
        };
        if uids.len() < 2 {
            return Err(bad("group_named needs at least 2 names"));
        }
        let k = uids.len();
        let op = json!({ "type": "groupNodes", "uids": uids, "uid": self.gen_id("grp"), "name": p.name });
        let n = session::apply_ops(&sess, &self.pool, vec![op], "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad(
                "group_named did not apply — the named items must share one parent",
            ));
        }
        Ok(format!(
            "grouped {k} items as \"{}\" → #indices renumbered, call get_document",
            p.name
        ))
    }

    #[tool(
        description = "Turn paths (by #index, 1+) into a reusable COMPONENT: they move into a `<g>` definition and a `<use>` instance replaces them (rendered in place). Then `stamp` copies instead of re-drawing — e.g. define a die once, stamp it. Editing a component part updates every instance. Paths must share one parent; `name` must be unique. Renumbers #indices — call get_document after."
    )]
    async fn create_component(
        &self,
        Parameters(p): Parameters<CreateComponentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
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
                    return Err(bad(format!("path #{i} has no tree uid")));
                }
                v.push(pe.uid.clone());
            }
            v
        };
        if uids.is_empty() {
            return Err(bad("create_component needs at least 1 path index"));
        }
        let op = json!({ "type": "createComponent", "members": uids, "name": p.name });
        let n = session::apply_ops(&sess, &self.pool, vec![op], "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad(
                "create_component did not apply — the paths must share one parent, and the name must be unique",
            ));
        }
        Ok(format!(
            "component \"{}\" created from {} paths → #indices renumbered, call get_document",
            p.name,
            uids.len()
        ))
    }

    #[tool(
        description = "Stamp a new instance of a component (by name) at optional x/y offset — a `<use>` reference, not a copy, so editing the component updates it too. Cheaper than re-drawing the shapes."
    )]
    async fn stamp(
        &self,
        Parameters(p): Parameters<StampParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let mut attributes = serde_json::Map::new();
        if let Some(x) = p.x {
            attributes.insert("x".into(), json!(x.to_string()));
        }
        if let Some(y) = p.y {
            attributes.insert("y".into(), json!(y.to_string()));
        }
        let op = json!({ "type": "stampInstance", "href": format!("#{}", p.component), "attributes": attributes });
        let n = session::apply_ops(&sess, &self.pool, vec![op], "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad("stamp did not apply (no active document?)"));
        }
        Ok(format!(
            "stamped an instance of \"{}\" → call get_document",
            p.component
        ))
    }

    #[tool(
        description = "List the document's components (name, uid, part count, instance count) — the reusable definitions you can `stamp`. To remove one (and cascade its instances), apply_op {type:\"deleteComponent\", uid}."
    )]
    async fn list_components(&self, ctx: RequestContext<RoleServer>) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let s = sess.lock().unwrap();
        let doc = s.editor.doc().ok_or_else(|| bad("no document"))?;
        let (comps, _) = component_info(doc);
        Ok(json!({ "components": comps }).to_string())
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

    #[tool(
        description = "Rotate a shape (by #index) `degrees` clockwise about its own centre — or an explicit cx/cy pivot (pass the same pivot to several shapes to rotate them as one). Use it to tilt, tumble, or orient a shape."
    )]
    async fn rotate(
        &self,
        Parameters(p): Parameters<RotateParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let op = json!({ "type": "rotatePath", "path": p.index, "degrees": p.degrees, "cx": p.cx, "cy": p.cy });
        let n = session::apply_ops(&sess, &self.pool, vec![op], "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad(
                "rotate did not apply (bad #index, deleted path, or non-finite degrees)",
            ));
        }
        Ok(format!("rotated #{} by {}°", p.index, p.degrees))
    }

    #[tool(
        description = "Flip a shape (by #index) — axis \"horizontal\" (left↔right) or \"vertical\" (top↕bottom) — mirroring it about its own centre."
    )]
    async fn flip(
        &self,
        Parameters(p): Parameters<FlipParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let a = p.axis.to_lowercase();
        let horizontal = a.starts_with('h') || a == "x";
        let op = json!({ "type": "flipPath", "path": p.index, "horizontal": horizontal });
        let n = session::apply_ops(&sess, &self.pool, vec![op], "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad("flip did not apply (bad #index or deleted path)"));
        }
        Ok(format!(
            "flipped #{} {}",
            p.index,
            if horizontal { "horizontal" } else { "vertical" }
        ))
    }

    #[tool(
        description = "Change a shape's z-order (by #index): `where` ∈ front | back | forward | backward. front/back move it all the way (bring-to-front / send-to-back); forward/backward one step. Renumbers #indices — call get_document after."
    )]
    async fn reorder(
        &self,
        Parameters(p): Parameters<ReorderParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let uid = {
            let s = sess.lock().unwrap();
            let doc = s.editor.doc().ok_or_else(|| bad("no document"))?;
            let pe = doc
                .paths
                .get(p.index)
                .ok_or_else(|| bad(format!("no path at #{}", p.index)))?;
            if pe.uid.is_empty() {
                return Err(bad("that shape has no tree uid (can't reorder)"));
            }
            pe.uid.clone()
        };
        let w = p.r#where.to_lowercase();
        let op = match w.as_str() {
            "front" => json!({ "type": "reorderNodeExtreme", "uid": uid, "front": true }),
            "back" => json!({ "type": "reorderNodeExtreme", "uid": uid, "front": false }),
            "forward" | "up" => json!({ "type": "reorderNode", "uid": uid, "forward": true }),
            "backward" | "down" => json!({ "type": "reorderNode", "uid": uid, "forward": false }),
            _ => return Err(bad("where must be front | back | forward | backward")),
        };
        let n = session::apply_ops(&sess, &self.pool, vec![op], "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad("reorder was a no-op (already at that edge?)"));
        }
        Ok(format!(
            "moved #{} {w} → #indices renumbered, call get_document",
            p.index
        ))
    }

    #[tool(
        description = "Give a shape (by #index) a soft DROP SHADOW — the one authorable effect (offset + blur + colour). Defaults: dx/dy 2, blur 2, black at 40%. Renders in the browser and in render_document. Re-applying replaces the shape's shadow."
    )]
    async fn drop_shadow(
        &self,
        Parameters(p): Parameters<DropShadowParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let op = json!({
            "type": "setDropShadow",
            "path": p.index,
            "dx": p.dx.unwrap_or(2.0),
            "dy": p.dy.unwrap_or(2.0),
            "blur": p.blur.unwrap_or(2.0),
            "color": p.color.clone().unwrap_or_else(|| "#000000".to_string()),
            "opacity": p.opacity.unwrap_or(0.4),
            "id": self.gen_id("shadow"),
        });
        let n = session::apply_ops(&sess, &self.pool, vec![op], "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad(
                "drop_shadow did not apply (bad #index or non-finite params)",
            ));
        }
        Ok(format!("drop shadow on #{}", p.index))
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
    use super::{component_info, find_by_name, render_png};

    #[test]
    fn component_info_lists_components_and_labels_parts() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><g id="die"><rect x="0" y="0" width="9" height="9"/><circle cx="2" cy="2" r="1"/></g></defs><use href="#die" x="10" y="10"/><use href="#die" x="40" y="10"/></svg>"##;
        let mut ed = nib_core::Editor::new();
        ed.load_source(svg).unwrap();
        let doc = ed.doc().unwrap();
        let (comps, part_comp) = component_info(doc);
        assert_eq!(comps.len(), 1, "one component: {comps:?}");
        assert_eq!(comps[0]["name"], "die");
        assert!(
            comps[0]["uid"].as_str().is_some_and(|u| !u.is_empty()),
            "component carries its <g> uid (addressable for deleteComponent): {:?}",
            comps[0]
        );
        assert_eq!(comps[0]["parts"], 2, "rect + circle"); // the def's two shapes
        assert_eq!(comps[0]["instances"], 2, "two <use>");
        // The def parts map back to the component name (outline labeling).
        assert_eq!(part_comp.len(), 2);
        assert!(part_comp.values().all(|v| v == "die"));
    }

    #[test]
    fn find_by_name_resolves_and_disambiguates() {
        // Two shapes a co-author might both call "hand" + an unrelated one.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <rect id="left-hand" x="10" y="40" width="8" height="8"/>
  <rect id="right-hand" x="70" y="40" width="8" height="8"/>
  <rect id="torso" x="40" y="40" width="10" height="30"/>
</svg>"##;
        let mut ed = nib_core::Editor::new();
        ed.load_source(svg).unwrap();
        let doc = ed.doc().unwrap();

        // "hand" is ambiguous — both hands come back (with index/uid/bounds) so the LLM can ask which.
        let hands = find_by_name(doc, "hand");
        assert_eq!(hands.len(), 2, "both hands: {hands:?}");
        assert!(hands.iter().all(|m| m["index"].is_number()
            && m["uid"].as_str().is_some_and(|u| !u.is_empty())
            && m["bounds"].is_object()));
        // An exact name resolves to one; a miss to none.
        assert_eq!(find_by_name(doc, "torso").len(), 1);
        assert_eq!(find_by_name(doc, "dragon").len(), 0);
        // Case-insensitive.
        assert_eq!(find_by_name(doc, "LEFT-HAND").len(), 1);
    }

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

    /// render_document is the LLM's "see your work" tool — for components it must resolve `<use href>`
    /// against a `<g>` definition, so BOTH instances paint. This is the one external dependency the
    /// components feature rides on (resvg's `<use>` resolution); pin it deterministically. viewBox is
    /// 200×100 rendered at target 200 → scale 1.0, so pixel coords == document coords.
    #[test]
    fn render_resolves_use_of_a_component_def_for_every_instance() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"><defs><g id="die"><rect x="0" y="0" width="40" height="40" fill="#3f86d4"/></g></defs><use href="#die" x="10" y="30"/><use href="#die" x="120" y="30"/></svg>"##;
        let png = render_png(svg, 200.0).expect("render");
        let pm = resvg::tiny_skia::Pixmap::decode_png(&png).expect("decode");
        assert_eq!((pm.width(), pm.height()), (200, 100));
        let px = |x: u32, y: u32| {
            let i = ((y * pm.width() + x) * 4) as usize;
            [pm.data()[i], pm.data()[i + 1], pm.data()[i + 2]]
        };
        // Instance A body ≈ (10,30)-(50,70); instance B ≈ (120,30)-(160,70).
        assert_ne!(px(30, 50), [255, 255, 255], "left <use> resolved + painted");
        assert_ne!(
            px(140, 50),
            [255, 255, 255],
            "right <use> resolved + painted"
        );
        // The gap between the two instances is untouched backdrop — not one giant fill.
        assert_eq!(px(85, 50), [255, 255, 255], "gap stays white");
    }
}
