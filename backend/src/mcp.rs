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

use nib_core::model::path::parse_path_d;
use nib_core::model::tree::RenderNode;
use nib_core::model::types::{Subpath, SvgDocument};

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
3. DRAW with draw_path (SVG `d` data) for anything curved or irregular — a shell, a wave, a \
silhouette; use add_shape only for true rectangles/ellipses/lines/polygons/stars. Then set_style, \
set_gradient, boolean_op, group, rename, rotate, flip, duplicate, reorder, apply_op (the full op \
vocabulary). Mutations return a ONE-LINE ack, not the document — that's deliberate; call \
get_document when you need the current #indices.\n\
3a. WORK IN BULK. rotate/flip/set_style/set_gradient/duplicate/reorder each take `index`, or \
`indices`, or a `name` — and a name that's a GROUP acts on every shape inside it. Tilting a \
nineteen-shape scene is ONE call naming the group, not nineteen. For a symmetric pair, duplicate \
then flip the copy about a cx/cy line rather than authoring mirrored coordinates by hand.\n\
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
not after every edit; pass a small `width` for a quick glance, or `around` a shape/group name to crop \
in on one part instead of re-rendering the whole drawing.\n\
6. Structural ops (group, boolean_op, reorder) RENUMBER #indices. Call get_document afterwards before \
you address paths by index again. Targeting by `name` is immune to this — one more reason to name things.\n\
6a. A ROTATED shape's document-axis bounds are not its size: get_document appends its own size and \
angle ('own 57.34×74.39 turned -25.69°') when it has been turned. Plan against those, and reuse the \
angle — tilting new work by the same amount about the same pivot is what makes it sit inside a \
tilted thing.\n\
7. TEXT is not a path: a label has no anchors, no #index, and no geometry to boolean or reshape — \
get_document lists labels separately. add_text places one; outline_text converts one (or all) into \
editable glyph outlines, after which it behaves like any other shape and renders identically \
everywhere, with no font needed. Outlining is destructive (the words stop being editable text), so do \
it when the shape matters more than the wording — and never as a shortcut for restyling a label, \
which apply_op setNodeAttr does non-destructively.\n\n\
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
    /// Whether the dev-auth bypass is on (a local MCP client with no token under `just dev`).
    dev_auth: bool,
    tool_router: ToolRouter<NibMcp>,
}

impl NibMcp {
    pub fn new(state: &AppState) -> Self {
        NibMcp {
            pool: state.pool.clone(),
            sessions: state.sessions.clone(),
            active: Arc::new(std::sync::Mutex::new(None)),
            next_id: Arc::new(AtomicU64::new(1)),
            dev_auth: state.cfg.dev_auth,
            tool_router: Self::tool_router(),
        }
    }

    /// Resolve the caller from the request's bearer token.
    ///
    /// Bearer only — deliberately. `/mcp` is same-origin with the SPA, so honouring the session
    /// cookie here would let any page in the browser drive the tool surface. It also means the
    /// MCP path never touches SSO: the personal token is the whole credential.
    async fn user(&self, ctx: &RequestContext<RoleServer>) -> Result<User, ErrorData> {
        let parts = ctx
            .extensions
            .get::<Parts>()
            .ok_or_else(|| bad("no request context"))?;
        auth::mcp_user(&self.pool, self.dev_auth, parts)
            .await
            .ok_or_else(|| {
                ErrorData::invalid_request(
                    "unauthorized — set Authorization: Bearer <token>".to_string(),
                    None,
                )
            })
    }

    /// The active project's session (the one `open_project`/`create_project` selected).
    ///
    /// `session::open` re-checks ownership against `user` on every call, so an `active` id set
    /// while holding one token can't be reused to reach that project with another.
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
    // Labels have no geometry, so they never appear in the path list below — say they exist, or the
    // words in the drawing are invisible to a reader of this outline.
    let labels = s.editor.text_infos();
    if !labels.is_empty() {
        let names: Vec<String> = labels
            .iter()
            .map(|l| {
                if l.name.is_empty() {
                    format!("\"{}\"", l.text.trim())
                } else {
                    format!("{} (\"{}\")", l.name, l.text.trim())
                }
            })
            .collect();
        lines.push(format!(
            "text labels (not paths — no #index): {} — outline_text converts one to editable geometry",
            names.join(", ")
        ));
    }
    for (i, p) in doc.paths.iter().enumerate() {
        if p.deleted {
            continue;
        }
        let bbox = match path_bounds(&p.subpaths, 0.0) {
            Some((x, y, w, h)) => format!("[{} {} {}×{}]", r(x), r(y), r(w), r(h)),
            None => "[empty]".to_string(),
        };
        // A turned shape's document-axis box is not its size: a 57×74 frame tilted 26° measures
        // 84×92 on the page. Report what it actually is — its own box and the angle — or a caller
        // reading only this line plans against a rectangle that doesn't exist.
        let turned = (p.box_angle != 0.0)
            .then(|| path_bounds(&p.subpaths, p.box_angle))
            .flatten()
            .map(|(_, _, w, h)| {
                format!(
                    " (own {}×{} turned {}°)",
                    r(w),
                    r(h),
                    r(p.box_angle.to_degrees())
                )
            })
            .unwrap_or_default();
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
            "#{i} {id} {bbox}{turned} fill {fill}{stroke} {nodes}n{hidden}{in_comp}"
        ));
    }
    lines.join("\n")
}

/// A path's bounds — **control handles included**, like the core's own `subpaths_bounds` —
/// optionally measured in a frame tilted by `angle` radians. Returns `(min_x, min_y, w, h)`.
///
/// Anchors alone would be cheaper but wrong for exactly the paths that matter most: an arc drawn
/// from two anchors and a pair of handles reports height 0, so a caller planning around it thinks
/// it's a flat line. A bezier stays inside its control hull, so this box always contains the ink
/// (it can be a little loose where a handle reaches past the curve).
fn path_bounds(subpaths: &[Subpath], angle: f64) -> Option<(f64, f64, f64, f64)> {
    let (cos, sin) = ((-angle).cos(), (-angle).sin());
    let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for sp in subpaths {
        for n in &sp.nodes {
            for pt in [Some(n.point), n.handle_in, n.handle_out]
                .into_iter()
                .flatten()
            {
                // Rotated *back* by the tilt, so the box is measured along the shape's own axes.
                let (x, y) = if angle == 0.0 {
                    (pt.x, pt.y)
                } else {
                    (pt.x * cos - pt.y * sin, pt.x * sin + pt.y * cos)
                };
                minx = minx.min(x);
                miny = miny.min(y);
                maxx = maxx.max(x);
                maxy = maxy.max(y);
            }
        }
    }
    (minx <= maxx).then_some((minx, miny, maxx - minx, maxy - miny))
}

/// The render node with this uid, anywhere in the tree.
fn find_render_node<'a>(nodes: &'a [RenderNode], uid: &str) -> Option<&'a RenderNode> {
    for n in nodes {
        if let RenderNode::Element {
            uid: u, children, ..
        } = n
        {
            if u == uid {
                return Some(n);
            }
            if let Some(hit) = find_render_node(children, uid) {
                return Some(hit);
            }
        }
    }
    None
}

/// Every editable path #index under a tree node — the node itself if it's a shape, or all the
/// shapes inside it if it's a `<g>`. This is what makes a *group* addressable: "rotate the crab"
/// rather than thirteen calls naming each leg.
fn node_path_indices(doc: &SvgDocument, uid: &str) -> Vec<usize> {
    let mut uids = vec![uid.to_string()];
    if let Some(tree) = doc.tree.as_ref()
        && let Some(node) = find_render_node(&tree.render_children(), uid)
    {
        collect_part_uids(node, &mut uids);
    }
    let mut out: Vec<usize> = uids
        .iter()
        .filter_map(|u| {
            doc.paths
                .iter()
                .position(|p| !p.deleted && !p.uid.is_empty() && &p.uid == u)
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Which paths a tool acts on: one `index`, several `indices`, or everything under a `name`.
///
/// `name` is the interesting one — it resolves a shape OR a `<g>` group, and a group expands to
/// every editable path inside it. Transforms used to be strictly one-#index-per-call, so tilting a
/// nineteen-shape scene was nineteen identical calls; naming the group makes it one.
fn resolve_targets(
    doc: &SvgDocument,
    index: Option<usize>,
    indices: Option<&[usize]>,
    name: Option<&str>,
) -> Result<Vec<usize>, ErrorData> {
    if let Some(n) = name.map(str::trim).filter(|n| !n.is_empty()) {
        let mut out: Vec<usize> = Vec::new();
        // A shape's name lives on the PATH. Only an imported or explicitly renamed node carries an
        // `id` attribute in the tree, so a freshly drawn "wave" is invisible to a tree-attr lookup
        // — matching `find`'s behaviour here is what makes naming reliable for things you drew.
        for (i, p) in doc.paths.iter().enumerate() {
            if !p.deleted && p.id.eq_ignore_ascii_case(n) {
                out.push(i);
            }
        }
        // ...and a tree node: a `<g>` expands to every shape inside it.
        for uid in uids_by_id(doc, n) {
            out.extend(node_path_indices(doc, &uid));
        }
        out.sort_unstable();
        out.dedup();
        if out.is_empty() {
            return Err(bad(format!(
                "no shape or group named \"{n}\" — call find or get_document"
            )));
        }
        return Ok(out);
    }
    let listed: Vec<usize> = match (indices, index) {
        (Some(v), _) if !v.is_empty() => v.to_vec(),
        (_, Some(i)) => vec![i],
        _ => return Err(bad("pass one of: index, indices, or name")),
    };
    for i in &listed {
        match doc.paths.get(*i) {
            None => return Err(bad(format!("no path at #{i}"))),
            Some(p) if p.deleted => return Err(bad(format!("#{i} is deleted"))),
            _ => {}
        }
    }
    Ok(listed)
}

/// "#3" / "#3, #7 and 2 more" — an ack that names what was touched without echoing the document.
fn listed(indices: &[usize]) -> String {
    match indices.len() {
        1 => format!("#{}", indices[0]),
        2 => format!("#{} and #{}", indices[0], indices[1]),
        n => format!("#{} … #{} ({n} shapes)", indices[0], indices[n - 1]),
    }
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
            if tag == "use"
                && let Some(h) = attrs.get("href").or_else(|| attrs.get("xlink:href"))
            {
                *uses
                    .entry(h.trim_start_matches('#').to_string())
                    .or_insert(0) += 1;
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
                    && ct == "g"
                    && let Some(id) = attrs.get("id")
                {
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
/// in-process. Labels render with the host's system fonts; on a `scratch` image there are none, so
/// `<text>` silently doesn't draw — one more reason to outline text before it leaves nib.
/// As `render_png`, but optionally cropped to `region` — `(x, y, w, h)` in the **rendered tree's**
/// coordinates (viewBox units, already offset by the viewBox origin).
///
/// The crop is done by rendering into a region-sized pixmap through a translate, not by rendering
/// the whole document and cutting it up: a tight crop of a large drawing would otherwise allocate
/// a pixmap scaled for the whole document — hundreds of megapixels for a close look at one corner.
fn render_png_region(
    svg: &str,
    target: f32,
    region: Option<(f32, f32, f32, f32)>,
) -> Result<Vec<u8>, String> {
    let opt = usvg::Options {
        fontdb: crate::fonts::database(),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(svg, &opt).map_err(|e| e.to_string())?;
    let size = tree.size();
    let (ox, oy, w, h) = region.unwrap_or((0.0, 0.0, size.width(), size.height()));
    if w <= 0.0 || h <= 0.0 {
        return Err("nothing to render there (zero-sized area)".into());
    }
    let scale = target / w.max(h);
    let pw = (w * scale).round().max(1.0) as u32;
    let ph = (h * scale).round().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(pw, ph).ok_or("pixmap allocation failed")?;
    pixmap.fill(tiny_skia::Color::WHITE);
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale).pre_translate(-ox, -oy),
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
pub struct RenameProjectParams {
    /// The project id to rename (from `list_projects`).
    pub id: i64,
    /// The new name.
    pub name: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DeleteProjectParams {
    /// The project id to delete (from `list_projects`).
    pub id: i64,
    /// The project's CURRENT name, exactly as `list_projects` reports it. Required as a
    /// confirmation: it makes an off-by-one id fail loudly instead of destroying the wrong
    /// document.
    pub name: String,
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
    /// For a `line` only: the far endpoint, so the line runs (x,y) → (x2,y2). Without it a line is
    /// the bounding box's ↘ diagonal, which can't express a ↗ one at all. Prefer these.
    #[serde(default)]
    pub x2: Option<f64>,
    #[serde(default)]
    pub y2: Option<f64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SetStyleParams {
    /// The path's #index (from get_document). One of index / indices / name is required.
    #[serde(default)]
    pub index: Option<usize>,
    /// Several #indices, acted on in one call.
    #[serde(default)]
    pub indices: Option<Vec<usize>>,
    /// A shape's or GROUP's name — a group expands to every shape inside it, so one call moves
    /// the whole thing. Prefer this: it's what your co-author says out loud.
    #[serde(default)]
    pub name: Option<String>,
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
    /// The path's #index (from get_document). One of index / indices / name is required.
    #[serde(default)]
    pub index: Option<usize>,
    /// Several #indices, acted on in one call.
    #[serde(default)]
    pub indices: Option<Vec<usize>>,
    /// A shape's or GROUP's name — a group expands to every shape inside it, so one call moves
    /// the whole thing. Prefer this: it's what your co-author says out loud.
    #[serde(default)]
    pub name: Option<String>,
    /// Rotation in degrees, clockwise (like SVG `rotate()`).
    pub degrees: f64,
    /// Optional pivot (viewBox units). Defaults to each shape's own bounding-box centre; pass a
    /// shared pivot (or target a group by `name`) to turn several shapes as one rigid body.
    #[serde(default)]
    pub cx: Option<f64>,
    #[serde(default)]
    pub cy: Option<f64>,
}

/// The one label `name` refers to — by its id or its words, exact match first, then a partial one.
/// Ambiguity is an error listing the candidates, not a guess: outlining is destructive, so picking
/// the wrong "title" costs the co-author their text.
fn pick_label<'a>(
    labels: &'a [nib_core::model::tree::TextInfo],
    name: &str,
) -> Result<&'a nib_core::model::tree::TextInfo, ErrorData> {
    let needle = name.trim().to_lowercase();
    let describe = |ls: &[&nib_core::model::tree::TextInfo]| {
        ls.iter()
            .map(|l| {
                if l.name.is_empty() {
                    format!("\"{}\"", l.text)
                } else {
                    format!("{} (\"{}\")", l.name, l.text)
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let exact: Vec<&_> = labels
        .iter()
        .filter(|l| l.name.to_lowercase() == needle || l.text.trim().to_lowercase() == needle)
        .collect();
    let matches = if exact.is_empty() {
        labels
            .iter()
            .filter(|l| {
                l.name.to_lowercase().contains(&needle) || l.text.to_lowercase().contains(&needle)
            })
            .collect::<Vec<_>>()
    } else {
        exact
    };
    match matches.len() {
        1 => Ok(matches[0]),
        0 => Err(bad(format!(
            "no label matches \"{name}\" — this document has: {}",
            describe(&labels.iter().collect::<Vec<_>>())
        ))),
        _ => Err(bad(format!(
            "\"{name}\" matches several labels: {} — ask which one, then use its exact id or words",
            describe(&matches)
        ))),
    }
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct OutlineTextParams {
    /// Which label to convert — its id, or the words it shows. Omit to convert every label.
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct AddTextParams {
    /// The text content, e.g. "Alea iacta est".
    pub text: String,
    /// Baseline position in viewBox units.
    pub x: f64,
    pub y: f64,
    /// Font size (px). Default 16.
    #[serde(default)]
    pub size: Option<f64>,
    /// Fill colour. Default black.
    #[serde(default)]
    pub fill: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct FlipParams {
    /// The path's #index (from get_document). One of index / indices / name is required.
    #[serde(default)]
    pub index: Option<usize>,
    /// Several #indices, acted on in one call.
    #[serde(default)]
    pub indices: Option<Vec<usize>>,
    /// A shape's or GROUP's name — a group expands to every shape inside it, so one call moves
    /// the whole thing. Prefer this: it's what your co-author says out loud.
    #[serde(default)]
    pub name: Option<String>,
    /// "horizontal" (left↔right) or "vertical" (top↕bottom).
    pub axis: String,
    /// Optional mirror line (viewBox units): `cx` is the vertical line x=cx for a horizontal flip,
    /// `cy` the horizontal line y=cy for a vertical one. Defaults to the shape's own centre — pass
    /// an explicit axis to mirror a copy ACROSS the drawing, which is how you build a symmetric
    /// pair (duplicate, then flip about the centreline).
    #[serde(default)]
    pub cx: Option<f64>,
    #[serde(default)]
    pub cy: Option<f64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReorderParams {
    /// The path's #index (from get_document). One of index / indices / name is required.
    #[serde(default)]
    pub index: Option<usize>,
    /// Several #indices, acted on in one call.
    #[serde(default)]
    pub indices: Option<Vec<usize>>,
    /// A shape's or GROUP's name — a group expands to every shape inside it, so one call moves
    /// the whole thing. Prefer this: it's what your co-author says out loud.
    #[serde(default)]
    pub name: Option<String>,
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
    /// Crop to one shape or group by name, padded a little — far cheaper than re-rendering the
    /// whole document to check one corner of it.
    #[serde(default)]
    pub around: Option<String>,
    /// Crop to an explicit region in viewBox units (all four required; overrides `around`).
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub w: Option<f64>,
    #[serde(default)]
    pub h: Option<f64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DrawPathParams {
    /// SVG path data — the `d` attribute. Absolute or relative, any of M/L/H/V/C/S/Q/T/A/Z; it is
    /// normalised to cubic anchors on the way in, exactly like an imported file.
    pub d: String,
    /// A descriptive id/name (do this — see the workflow).
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub fill: Option<String>,
    #[serde(default)]
    pub stroke: Option<String>,
    #[serde(default, rename = "strokeWidth")]
    pub stroke_width: Option<f64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DuplicateParams {
    /// The path's #index (from get_document). One of index / indices / name is required.
    #[serde(default)]
    pub index: Option<usize>,
    #[serde(default)]
    pub indices: Option<Vec<usize>>,
    /// A shape's or GROUP's name — a group copies every shape inside it.
    #[serde(default)]
    pub name: Option<String>,
    /// Offset for the copy (viewBox units). Default 0,0 — an exact overlay, which is what you want
    /// before mirroring it into place.
    #[serde(default)]
    pub dx: Option<f64>,
    #[serde(default)]
    pub dy: Option<f64>,
    /// Name for the copy. A single copy takes it verbatim; several get "-2", "-3", … appended.
    /// Defaults to the original's name with "-copy".
    #[serde(rename = "newName", default)]
    pub new_name: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SetGradientParams {
    /// The path's #index (from get_document). One of index / indices / name is required.
    #[serde(default)]
    pub index: Option<usize>,
    #[serde(default)]
    pub indices: Option<Vec<usize>>,
    /// A shape's or GROUP's name — every shape inside it takes the same gradient.
    #[serde(default)]
    pub name: Option<String>,
    /// "linear" (default) or "radial".
    #[serde(default)]
    pub kind: Option<String>,
    /// Two or more colour stops, in order. Each is `{"offset":0..1,"color":"#rrggbb","opacity":0..1}`
    /// — `opacity` optional. Two stops is the common case: a sky, a sea, a sheen.
    pub stops: Vec<serde_json::Value>,
    /// LINEAR: direction in degrees clockwise, 0 = left→right, 90 = top→bottom. Default 90.
    #[serde(default)]
    pub angle: Option<f64>,
    /// RADIAL: centre + radius as fractions of the shape's own box (0..1). Default 0.5/0.5/0.5.
    #[serde(default)]
    pub cx: Option<f64>,
    #[serde(default)]
    pub cy: Option<f64>,
    #[serde(default)]
    pub r: Option<f64>,
    /// Paint the stroke instead of the fill.
    #[serde(default)]
    pub stroke: Option<bool>,
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
        description = "Rename one of your projects. This renames the PROJECT, not a shape inside it — use `rename` for a shape."
    )]
    async fn rename_project(
        &self,
        Parameters(p): Parameters<RenameProjectParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let name = p.name.trim();
        if name.is_empty() {
            return Err(bad("name must not be empty"));
        }
        match db::rename_project(&self.pool, user.id, p.id, name).await {
            Ok(true) => Ok(format!("renamed project {} to \"{name}\"", p.id)),
            Ok(false) => Err(bad(format!("no such project: {}", p.id))),
            Err(e) => Err(bad(e.to_string())),
        }
    }

    #[tool(
        description = "PERMANENTLY delete one of your projects and its artwork. There is no undo and no trash. Only call this when the human has asked for this specific project to be deleted — never to tidy up, and never on your own initiative. You must pass the project's current `name` alongside its `id`; they have to match, so call list_projects first and don't guess."
    )]
    async fn delete_project(
        &self,
        Parameters(p): Parameters<DeleteProjectParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        // Confirm against the stored name before destroying anything: a hallucinated or stale id
        // that happens to exist would otherwise delete somebody's drawing silently.
        let project = db::get_project(&self.pool, user.id, p.id)
            .await
            .map_err(|e| bad(e.to_string()))?
            .ok_or_else(|| bad(format!("no such project: {}", p.id)))?;
        if project.name != p.name {
            return Err(bad(format!(
                "name mismatch — project {} is called \"{}\", not \"{}\". Nothing deleted; call list_projects and retry with the exact name.",
                p.id, project.name, p.name
            )));
        }
        if !db::delete_project(&self.pool, user.id, p.id)
            .await
            .map_err(|e| bad(e.to_string()))?
        {
            return Err(bad(format!("no such project: {}", p.id)));
        }
        session::close(&self.sessions, p.id);
        // Don't leave the connection pointed at a project that no longer exists.
        let mut active = self.active.lock().unwrap();
        if *active == Some(p.id) {
            *active = None;
        }
        Ok(format!("deleted project {} (\"{}\")", p.id, project.name))
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
        description = "Render the active project to a PNG and return it as an image, so you can SEE the drawing and verify it reads correctly (get_document gives structure; this gives pixels). Image-heavy — render at checkpoints, not after every edit; pass a smaller `width` for a cheap glance, or `around` a shape/group name (or an explicit x/y/w/h) to crop in on just that part instead of re-rendering everything. Composited on white; labels render with the server's fonts, which may substitute a different face than the author saw."
    )]
    async fn render_document(
        &self,
        Parameters(p): Parameters<RenderParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let (svg, region) = {
            let s = sess.lock().unwrap();
            let svg = s.editor.to_svg();
            let doc = s.editor.doc().ok_or_else(|| bad("no document"))?;
            // The rendered tree's origin is the viewBox origin, so a caller's viewBox coordinates
            // shift by it before they mean pixels.
            let (ox, oy) = (doc.view_box.min_x, doc.view_box.min_y);
            let region = match (p.x, p.y, p.w, p.h) {
                (Some(x), Some(y), Some(w), Some(h)) => Some((x - ox, y - oy, w, h)),
                _ => match p.around.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                    Some(n) => {
                        let targets = resolve_targets(doc, None, None, Some(n))?;
                        let (mut minx, mut miny, mut maxx, mut maxy) =
                            (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                        for i in &targets {
                            if let Some((x, y, w, h)) = path_bounds(&doc.paths[*i].subpaths, 0.0) {
                                minx = minx.min(x);
                                miny = miny.min(y);
                                maxx = maxx.max(x + w);
                                maxy = maxy.max(y + h);
                            }
                        }
                        if minx > maxx {
                            return Err(bad(format!("\"{n}\" has no geometry to frame")));
                        }
                        // A crop flush to the ink reads as clipped; a tenth of a margin (and a
                        // floor, for a shape that is a single point) shows it in its setting.
                        let pad = ((maxx - minx).max(maxy - miny) * 0.1).max(1.0);
                        Some((
                            minx - ox - pad,
                            miny - oy - pad,
                            (maxx - minx) + pad * 2.0,
                            (maxy - miny) + pad * 2.0,
                        ))
                    }
                    None => None,
                },
            };
            (svg, region)
        };
        let target = p.width.unwrap_or(512.0).clamp(128.0, 1024.0) as f32;
        let region = region.map(|(x, y, w, h)| (x as f32, y as f32, w as f32, h as f32));
        let png = render_png_region(&svg, target, region)
            .map_err(|e| bad(format!("render failed: {e}")))?;
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
        description = "Add a shape by bounding box. shape ∈ {ellipse, rect, line, polygon, star}; x/y/w/h in viewBox units; optional fill/stroke colours. For a `line`, pass x2/y2 for the far endpoint — by bounding box alone a line can only be the ↘ diagonal. Pass `name` to give it a descriptive id (do this — see the workflow). For anything curved or irregular, use draw_path instead. Returns the new path's #index."
    )]
    async fn add_shape(
        &self,
        Parameters(p): Parameters<AddShapeParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let mut spec = shape_spec(&p.shape, p.x, p.y, p.w, p.h, p.radius.unwrap_or(0.0))
            .ok_or_else(|| bad(format!("unknown shape: {}", p.shape)))?;
        // A line given real endpoints runs (x,y) → (x2,y2); the bounding box can only ever draw
        // its ↘ diagonal, so "/" was unreachable without adding-then-flipping.
        if spec["shape"] == "line"
            && let (Some(x2), Some(y2)) = (p.x2, p.y2)
        {
            spec["x1"] = json!(x2);
            spec["y1"] = json!(y2);
        }
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
        description = "Draw a path from SVG path data — the `d` attribute. THE tool for anything curved or irregular: a shell, a wave, a leaf, a silhouette. Absolute or relative, any of M/L/H/V/C/S/Q/T/A/Z, multiple subpaths fine; it normalises to cubic anchors exactly like an imported file, so the human can then drag its points. Prefer add_shape only for true rectangles/ellipses/stars. Returns the new path's #index."
    )]
    async fn draw_path(
        &self,
        Parameters(p): Parameters<DrawPathParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let subpaths = parse_path_d(&p.d);
        if subpaths.is_empty() {
            return Err(bad(
                "could not parse `d` as SVG path data (it must start with a moveto — e.g. \"M 10 10 C …\")",
            ));
        }
        let mut attrs = serde_json::Map::new();
        // A path with neither fill nor stroke renders nothing at all, which reads as the tool
        // having failed. Default to a visible fill, as `<path>` itself does.
        attrs.insert(
            "fill".into(),
            json!(p.fill.clone().unwrap_or_else(|| "#000000".into())),
        );
        if let Some(st) = &p.stroke {
            attrs.insert("stroke".into(), json!(st));
        }
        if let Some(w) = p.stroke_width {
            attrs.insert("stroke-width".into(), json!(w.to_string()));
        }
        let id = p
            .name
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.gen_id("path"));
        let op = json!({ "type": "addPath", "id": id, "subpaths": subpaths, "attributes": attrs });
        session::apply_ops(&sess, &self.pool, vec![op], "mcp").map_err(bad)?;
        let s = sess.lock().unwrap();
        let (idx, rid) = tail(&s);
        let nodes: usize = subpaths.iter().map(|sp| sp.nodes.len()).sum();
        Ok(format!(
            "drew #{idx} \"{rid}\" · {nodes} nodes · {} paths",
            count_paths(&s)
        ))
    }

    #[tool(
        description = "Copy one shape (`index`), several (`indices`), or a whole group (`name`), optionally offset by dx/dy. The copies land on top, in order. Pair it with flip's cx/cy to build a symmetric half: duplicate, then mirror the copy about the centreline — fewer calls than authoring mirrored coordinates, and exactly symmetric. Renumbers nothing (copies append), but call get_document for the new #indices."
    )]
    async fn duplicate(
        &self,
        Parameters(p): Parameters<DuplicateParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let (dx, dy) = (p.dx.unwrap_or(0.0), p.dy.unwrap_or(0.0));
        if !dx.is_finite() || !dy.is_finite() {
            return Err(bad("dx/dy must be finite"));
        }
        let ops = {
            let s = sess.lock().unwrap();
            let doc = s.editor.doc().ok_or_else(|| bad("no document"))?;
            let targets = resolve_targets(doc, p.index, p.indices.as_deref(), p.name.as_deref())?;
            let many = targets.len() > 1;
            let mut ops = Vec::new();
            for (n, i) in targets.iter().enumerate() {
                let src = &doc.paths[*i];
                let mut subpaths = src.subpaths.clone();
                for sp in &mut subpaths {
                    for node in &mut sp.nodes {
                        node.point.x += dx;
                        node.point.y += dy;
                        for hv in [node.handle_in.as_mut(), node.handle_out.as_mut()]
                            .into_iter()
                            .flatten()
                        {
                            hv.x += dx;
                            hv.y += dy;
                        }
                    }
                }
                // The copy carries the style it looked like, not the source's raw attrs: an
                // imported shape keeps its edits in `style_override`, and a copy that dropped
                // them would come out a different colour from the thing it copied.
                let mut attrs = src.attributes.clone().unwrap_or_default();
                if let Some(over) = &src.style_override {
                    for (k, v) in over {
                        attrs.insert(k.clone(), v.clone());
                    }
                }
                let base = p
                    .new_name
                    .clone()
                    .unwrap_or_else(|| format!("{}-copy", src.id));
                let id = if many {
                    format!("{base}-{}", n + 1)
                } else {
                    base
                };
                ops.push(
                    json!({ "type": "addPath", "id": id, "subpaths": subpaths, "attributes": attrs }),
                );
            }
            ops
        };
        let made = ops.len();
        let n = session::apply_ops(&sess, &self.pool, ops, "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad("nothing was copied"));
        }
        let s = sess.lock().unwrap();
        let (idx, _) = tail(&s);
        Ok(format!(
            "copied {made} shape(s) → #{}…#{idx} · {} paths · call get_document",
            idx + 1 - made,
            count_paths(&s)
        ))
    }

    #[tool(
        description = "Fill one shape (`index`), several (`indices`), or a group (`name`) with a LINEAR or RADIAL gradient — a sky, a sea, a metal sheen. `stops` is 2+ {offset,color,opacity?} in order; `angle` aims a linear one (0 = left→right, 90 = top→bottom), cx/cy/r place a radial one within each shape's own box. The gradient becomes a real `<defs>` entry the human can then drag stops on."
    )]
    async fn set_gradient(
        &self,
        Parameters(p): Parameters<SetGradientParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        if p.stops.len() < 2 {
            return Err(bad("a gradient needs at least 2 stops"));
        }
        let mut stops = Vec::with_capacity(p.stops.len());
        for (i, st) in p.stops.iter().enumerate() {
            let offset = st
                .get("offset")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| bad(format!("stop {i} has no numeric `offset` (0..1)")))?;
            let color = st
                .get("color")
                .and_then(|v| v.as_str())
                .ok_or_else(|| bad(format!("stop {i} has no `color`")))?;
            let mut o = json!({ "offset": offset.clamp(0.0, 1.0), "color": color });
            if let Some(a) = st.get("opacity").and_then(|v| v.as_f64()) {
                o["opacity"] = json!(a.clamp(0.0, 1.0));
            }
            stops.push(o);
        }
        let kind = p.kind.as_deref().unwrap_or("linear").to_lowercase();
        if kind != "linear" && kind != "radial" {
            return Err(bad("kind must be \"linear\" or \"radial\""));
        }
        // Direction as a vector across the shape's own box, centred — the same parameterisation
        // the browser's angle slider writes, so the human can pick it up and turn it.
        let t = p.angle.unwrap_or(90.0).to_radians();
        let (hx, hy) = (t.cos() / 2.0, t.sin() / 2.0);
        // cos(90°) is 6e-17, not 0 — rounded, or the exported def reads "0.49999999999999994".
        let q = |v: f64| (v * 10_000.0).round() / 10_000.0;
        let id = self.gen_id("grad");
        let gradient = json!({
            "id": id, "kind": kind, "stops": stops,
            "x1": q(0.5 - hx), "y1": q(0.5 - hy), "x2": q(0.5 + hx), "y2": q(0.5 + hy),
            "cx": p.cx.unwrap_or(0.5), "cy": p.cy.unwrap_or(0.5), "r": p.r.unwrap_or(0.5),
        });
        let targets = {
            let s = sess.lock().unwrap();
            let doc = s.editor.doc().ok_or_else(|| bad("no document"))?;
            resolve_targets(doc, p.index, p.indices.as_deref(), p.name.as_deref())?
        };
        let key = if p.stroke.unwrap_or(false) {
            "stroke"
        } else {
            "fill"
        };
        let mut ops = vec![json!({ "type": "setGradient", "gradient": gradient })];
        for i in &targets {
            ops.push(
                json!({ "type": "setStyle", "path": i, "key": key, "value": format!("url(#{id})") }),
            );
        }
        let n = session::apply_ops(&sess, &self.pool, ops, "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad("the gradient did not apply"));
        }
        Ok(format!(
            "{kind} gradient \"{id}\" → {key} of {}",
            listed(&targets)
        ))
    }

    #[tool(
        description = "Add a text label at (x, y) — a `<text>` element (baseline at y). Optional font size + fill. Note: text isn't a path, so it has no #index and won't appear in get_document's path list (labels are listed separately); outline_text converts it to editable geometry when you need to reshape it."
    )]
    async fn add_text(
        &self,
        Parameters(p): Parameters<AddTextParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let mut attributes = serde_json::Map::new();
        attributes.insert(
            "font-size".into(),
            json!(p.size.unwrap_or(16.0).to_string()),
        );
        attributes.insert(
            "fill".into(),
            json!(p.fill.clone().unwrap_or_else(|| "#000000".to_string())),
        );
        let op = json!({ "type": "addText", "x": p.x, "y": p.y, "text": p.text, "attributes": attributes });
        let n = session::apply_ops(&sess, &self.pool, vec![op], "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad("add_text did not apply (no active document?)"));
        }
        Ok(format!("added text \"{}\" at ({}, {})", p.text, p.x, p.y))
    }

    #[tool(
        description = "Convert text labels to outlines: the words become an editable path of their glyph shapes, which renders identically everywhere (no font needed) and can then be node-edited, boolean'd or offset like any shape. Destructive — the label stops being text — and one undo step per label. Pass `name` to convert one (matches its id or its words); omit it to convert every label. Renumbers #indices, so call get_document after."
    )]
    async fn outline_text(
        &self,
        Parameters(p): Parameters<OutlineTextParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;

        // Shape first (a read), then apply — so the lock isn't held across the session write, and a
        // font that can't be found is reported before anything in the document changes.
        let (ops, labels, overridden) = {
            let s = sess.lock().unwrap();
            let all = s.editor.text_infos();
            if all.is_empty() {
                return Err(bad("no labels in this document to outline"));
            }
            let targets = match &p.name {
                Some(name) => vec![pick_label(&all, name)?],
                None => all.iter().collect(),
            };
            let mut ops = Vec::new();
            let mut labels = Vec::new();
            // Families a `<tspan>` asked for that the label's own font will override — one font
            // shapes a label, so those runs change typeface and the caller should hear it.
            let mut overridden: Vec<String> = Vec::new();
            for info in targets {
                let (font, index) = crate::fonts::face_for(info).ok_or_else(|| {
                    bad(format!(
                        "no font on this server matches \"{}\" — outline it in the browser, which can use your installed fonts",
                        info.family
                    ))
                })?;
                let d = s
                    .editor
                    .text_outline_d(&info.uid, &font, index)
                    .ok_or_else(|| bad(format!("\"{}\" produced no outlines", info.text)))?;
                ops.push(json!({ "type": "textToPath", "uid": info.uid, "d": d }));
                labels.push(info.text.clone());
                for family in info.foreign_families() {
                    if !overridden.iter().any(|f| f.eq_ignore_ascii_case(&family)) {
                        overridden.push(family);
                    }
                }
            }
            (ops, labels, overridden)
        };

        let n = session::apply_ops(&sess, &self.pool, ops, "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad("outline_text did not apply (no active document?)"));
        }
        let s = sess.lock().unwrap();
        // One font shapes a whole label, so a tspan that asked for another one changed typeface.
        // Say which, rather than leaving the model to notice from a render.
        let note = if overridden.is_empty() {
            String::new()
        } else {
            format!(
                " · runs asking for {} were shaped in the label's own font",
                overridden.join(", ")
            )
        };
        Ok(format!(
            "outlined {n} label(s) [{}] · now editable paths{note} · #indices renumbered, call get_document · {} paths",
            labels.join(", "),
            count_paths(&s)
        ))
    }

    #[tool(
        description = "Set paint/stroke on one shape (`index`), several (`indices`), or every shape in a group (`name`): any of fill, stroke, strokeWidth, opacity."
    )]
    async fn set_style(
        &self,
        Parameters(p): Parameters<SetStyleParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let targets = {
            let s = sess.lock().unwrap();
            let doc = s.editor.doc().ok_or_else(|| bad("no document"))?;
            resolve_targets(doc, p.index, p.indices.as_deref(), p.name.as_deref())?
        };
        let mut pairs: Vec<(&str, String)> = Vec::new();
        if let Some(f) = &p.fill {
            pairs.push(("fill", f.clone()));
        }
        if let Some(st) = &p.stroke {
            pairs.push(("stroke", st.clone()));
        }
        if let Some(w) = p.stroke_width {
            pairs.push(("stroke-width", w.to_string()));
        }
        if let Some(o) = p.opacity {
            pairs.push(("opacity", o.to_string()));
        }
        if pairs.is_empty() {
            return Err(bad(
                "nothing to set — pass at least one of fill/stroke/strokeWidth/opacity",
            ));
        }
        let ops: Vec<_> = targets
            .iter()
            .flat_map(|i| {
                pairs.iter().map(
                    move |(k, v)| json!({ "type": "setStyle", "path": i, "key": k, "value": v }),
                )
            })
            .collect();
        let n = session::apply_ops(&sess, &self.pool, ops, "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad("no style applied (bad #index?)"));
        }
        Ok(format!("styled {}", listed(&targets)))
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
        description = "Rotate `degrees` clockwise: one shape (`index`), several (`indices`), or a whole group (`name`). Turns about each shape's own centre unless you pass a cx/cy pivot — a group with a shared pivot turns as one rigid body, which is how you tilt a whole scene in one call."
    )]
    async fn rotate(
        &self,
        Parameters(p): Parameters<RotateParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let targets = {
            let s = sess.lock().unwrap();
            let doc = s.editor.doc().ok_or_else(|| bad("no document"))?;
            resolve_targets(doc, p.index, p.indices.as_deref(), p.name.as_deref())?
        };
        // Several shapes with no explicit pivot would each spin in place, which is never what
        // "rotate the crab" means — default a multi-target turn to the selection's shared centre.
        let (cx, cy) = match (p.cx, p.cy) {
            (Some(x), Some(y)) => (Some(x), Some(y)),
            _ if targets.len() > 1 => {
                let s = sess.lock().unwrap();
                let doc = s.editor.doc().ok_or_else(|| bad("no document"))?;
                let (mut minx, mut miny, mut maxx, mut maxy) =
                    (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                for i in &targets {
                    if let Some((x, y, w, h)) = path_bounds(&doc.paths[*i].subpaths, 0.0) {
                        minx = minx.min(x);
                        miny = miny.min(y);
                        maxx = maxx.max(x + w);
                        maxy = maxy.max(y + h);
                    }
                }
                if minx <= maxx {
                    (Some((minx + maxx) / 2.0), Some((miny + maxy) / 2.0))
                } else {
                    (None, None)
                }
            }
            _ => (p.cx, p.cy),
        };
        let ops: Vec<_> = targets
            .iter()
            .map(|i| {
                json!({ "type": "rotatePath", "path": i, "degrees": p.degrees, "cx": cx, "cy": cy })
            })
            .collect();
        let n = session::apply_ops(&sess, &self.pool, ops, "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad(
                "rotate did not apply (deleted path, or non-finite degrees)",
            ));
        }
        Ok(format!("rotated {} by {}°", listed(&targets), p.degrees))
    }

    #[tool(
        description = "Mirror one shape (`index`), several (`indices`), or a group (`name`) — axis \"horizontal\" (left↔right) or \"vertical\" (top↕bottom). About each shape's own centre by default; pass cx/cy to mirror about a line instead, which is how you make a symmetric pair: duplicate, then flip the copy about the drawing's centreline."
    )]
    async fn flip(
        &self,
        Parameters(p): Parameters<FlipParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let user = self.user(&ctx).await?;
        let sess = self.active_session(&user).await?;
        let targets = {
            let s = sess.lock().unwrap();
            let doc = s.editor.doc().ok_or_else(|| bad("no document"))?;
            resolve_targets(doc, p.index, p.indices.as_deref(), p.name.as_deref())?
        };
        let a = p.axis.to_lowercase();
        let horizontal = a.starts_with('h') || a == "x";
        // A horizontal flip only needs `cx` and a vertical one only `cy`, but the op takes the
        // pivot as a pair and falls back to the shape's own centre unless BOTH are given — so a
        // caller passing just `cx` would get a silent mirror-in-place instead of the mirror they
        // asked for. Fill the axis they left out from each shape's own centre.
        let ops: Vec<_> = {
            let s = sess.lock().unwrap();
            let doc = s.editor.doc().ok_or_else(|| bad("no document"))?;
            targets
                .iter()
                .map(|i| {
                    let own = path_bounds(&doc.paths[*i].subpaths, 0.0);
                    let (cx, cy) = match (p.cx, p.cy) {
                        (None, None) => (None, None),
                        (cx, cy) => own.map_or((None, None), |(x, y, w, h)| {
                            (
                                Some(cx.unwrap_or(x + w / 2.0)),
                                Some(cy.unwrap_or(y + h / 2.0)),
                            )
                        }),
                    };
                    json!({ "type": "flipPath", "path": i, "horizontal": horizontal, "cx": cx, "cy": cy })
                })
                .collect()
        };
        let n = session::apply_ops(&sess, &self.pool, ops, "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad("flip did not apply (deleted path?)"));
        }
        Ok(format!(
            "flipped {} {}",
            listed(&targets),
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
        // A group is reordered as the group NODE, not its members — moving thirteen crab parts to
        // the front one by one would interleave them with whatever they passed.
        let (uids, label) = {
            let s = sess.lock().unwrap();
            let doc = s.editor.doc().ok_or_else(|| bad("no document"))?;
            match p.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                Some(n) => {
                    // A group moves as its own node (its children keep their order); a plain
                    // shape has no such node, so fall back to the paths the name resolves to.
                    let mut uids = uids_by_id(doc, n);
                    if uids.is_empty() {
                        uids = resolve_targets(doc, None, None, Some(n))?
                            .iter()
                            .map(|i| doc.paths[*i].uid.clone())
                            .filter(|u| !u.is_empty())
                            .collect();
                    }
                    if uids.is_empty() {
                        return Err(bad(format!("no shape or group named \"{n}\"")));
                    }
                    (uids, format!("\"{n}\""))
                }
                None => {
                    let targets = resolve_targets(doc, p.index, p.indices.as_deref(), None)?;
                    let mut uids = Vec::new();
                    for i in &targets {
                        let pe = &doc.paths[*i];
                        if pe.uid.is_empty() {
                            return Err(bad(format!("#{i} has no tree uid (can't reorder)")));
                        }
                        uids.push(pe.uid.clone());
                    }
                    (uids, listed(&targets))
                }
            }
        };
        let w = p.r#where.to_lowercase();
        // Send-to-back reverses: the last one moved ends up outermost, so walk the list backwards
        // to preserve the group's internal order at the destination.
        let ordered: Vec<&String> = if w == "back" || w == "backward" || w == "down" {
            uids.iter().rev().collect()
        } else {
            uids.iter().collect()
        };
        let mut ops = Vec::new();
        for uid in ordered {
            ops.push(match w.as_str() {
                "front" => json!({ "type": "reorderNodeExtreme", "uid": uid, "front": true }),
                "back" => json!({ "type": "reorderNodeExtreme", "uid": uid, "front": false }),
                "forward" | "up" => json!({ "type": "reorderNode", "uid": uid, "forward": true }),
                "backward" | "down" => {
                    json!({ "type": "reorderNode", "uid": uid, "forward": false })
                }
                _ => return Err(bad("where must be front | back | forward | backward")),
            });
        }
        let n = session::apply_ops(&sess, &self.pool, ops, "mcp").map_err(bad)?;
        if n == 0 {
            return Err(bad("reorder was a no-op (already at that edge?)"));
        }
        Ok(format!(
            "moved {label} {w} → #indices renumbered, call get_document"
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
    use super::{
        component_info, find_by_name, node_path_indices, path_bounds, pick_label,
        render_png_region, resolve_targets, uids_by_id,
    };

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

    /// A scene with a `<g>` in it — the shape of document these tools are for.
    const SCENE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <rect id="frame" x="5" y="5" width="90" height="90" fill="none" stroke="#000"/>
  <g id="crab">
    <ellipse id="crab-body" cx="50" cy="60" rx="10" ry="6" fill="#d63a2c"/>
    <ellipse id="crab-claw" cx="32" cy="55" rx="4" ry="3" fill="#d63a2c"/>
  </g>
</svg>"##;

    fn scene() -> nib_core::Editor {
        let mut ed = nib_core::Editor::new();
        ed.load_source(SCENE).unwrap();
        ed
    }

    #[test]
    fn a_group_name_resolves_to_every_shape_inside_it() {
        let ed = scene();
        let doc = ed.doc().unwrap();

        // The whole point: "crab" is a <g>, and naming it reaches both its shapes — one call
        // instead of one per limb.
        let crab = resolve_targets(doc, None, None, Some("crab")).unwrap();
        assert_eq!(crab.len(), 2, "both crab parts: {crab:?}");

        // A leaf shape's name resolves to just itself.
        let body = resolve_targets(doc, None, None, Some("crab-body")).unwrap();
        assert_eq!(body.len(), 1);
        assert!(crab.contains(&body[0]));

        // Explicit indices and a single index still work, and are validated.
        assert_eq!(
            resolve_targets(doc, None, Some(&[0, 1]), None).unwrap(),
            vec![0, 1]
        );
        assert_eq!(resolve_targets(doc, Some(1), None, None).unwrap(), vec![1]);
        assert!(resolve_targets(doc, Some(99), None, None).is_err());
        assert!(resolve_targets(doc, None, None, Some("seahorse")).is_err());
        // No target at all is a clear error, not a silent no-op over everything.
        assert!(resolve_targets(doc, None, None, None).is_err());
    }

    #[test]
    fn a_group_uid_expands_to_its_shapes_but_a_shape_uid_is_itself() {
        let ed = scene();
        let doc = ed.doc().unwrap();
        let group = uids_by_id(doc, "crab");
        assert_eq!(group.len(), 1, "one <g> named crab");
        assert_eq!(node_path_indices(doc, &group[0]).len(), 2);

        let leaf = uids_by_id(doc, "crab-claw");
        assert_eq!(node_path_indices(doc, &leaf[0]).len(), 1);

        // An unknown uid reaches nothing rather than panicking or matching everything.
        assert!(node_path_indices(doc, "no-such-uid").is_empty());
    }

    #[test]
    fn a_drawn_shapes_name_resolves_even_though_it_has_no_tree_id() {
        // Regression: a shape you just drew keeps its name on the PathElement, and only an
        // imported or renamed node carries an `id` attribute in the tree. Resolving names through
        // tree attributes alone made every freshly drawn shape unaddressable by the name the tool
        // had just acknowledged — "drew #0 \"wave\"" followed by "no shape named wave".
        let mut ed = nib_core::Editor::new();
        ed.load_source(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"></svg>"##)
            .unwrap();
        assert!(ed.apply(&nib_core::ops::Op::AddPath {
            id: "wave".into(),
            subpaths: super::parse_path_d("M 10 80 C 20 10, 60 10, 70 80"),
            attributes: Default::default(),
            uid: Some("u-wave".into()),
        }));
        let doc = ed.doc().unwrap();
        assert!(
            uids_by_id(doc, "wave").is_empty(),
            "it genuinely has no tree id — that's the trap"
        );
        assert_eq!(
            resolve_targets(doc, None, None, Some("wave")).unwrap(),
            vec![0],
            "...and the name still resolves"
        );
        // Case-insensitively, like find.
        assert_eq!(
            resolve_targets(doc, None, None, Some("WAVE")).unwrap(),
            vec![0]
        );
    }

    #[test]
    fn a_flip_pivot_needs_both_axes_which_is_why_the_tool_fills_one_in() {
        // The op takes its pivot as a pair and falls back to the shape's own centre unless BOTH
        // are given, so `flip { cx }` alone would silently mirror in place. Pinning that here
        // because it's the reason `flip` computes the axis the caller left out.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <rect id="mark" x="10" y="10" width="20" height="10"/>
</svg>"##;
        let left = |ed: &nib_core::Editor| {
            path_bounds(&ed.doc().unwrap().paths[0].subpaths, 0.0)
                .unwrap()
                .0
        };

        let mut half = nib_core::Editor::new();
        half.load_source(svg).unwrap();
        assert!(half.apply(&nib_core::ops::Op::FlipPath {
            path: 0,
            horizontal: true,
            cx: Some(50.0),
            cy: None,
        }));
        assert!(
            (left(&half) - 10.0).abs() < 1e-6,
            "cx alone was ignored — mirrored in place at {}",
            left(&half)
        );

        let mut both = nib_core::Editor::new();
        both.load_source(svg).unwrap();
        assert!(both.apply(&nib_core::ops::Op::FlipPath {
            path: 0,
            horizontal: true,
            cx: Some(50.0),
            cy: Some(15.0),
        }));
        // Mirrored about x=50: [10,30] → [70,90].
        assert!(
            (left(&both) - 70.0).abs() < 1e-6,
            "mirrored across the line: {}",
            left(&both)
        );
    }

    #[test]
    fn oriented_bounds_report_a_turned_shape_own_size() {
        // A 40×20 rect turned 30°: its document-axis box grows, its own box must not.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <rect id="board" x="30" y="40" width="40" height="20"/>
</svg>"##;
        let mut ed = nib_core::Editor::new();
        ed.load_source(svg).unwrap();
        let angle = 30f64.to_radians();
        assert!(ed.apply(&nib_core::ops::Op::RotatePath {
            path: 0,
            degrees: 30.0,
            cx: None,
            cy: None,
        }));
        let doc = ed.doc().unwrap();
        let sub = &doc.paths[0].subpaths;

        let (_, _, aw, ah) = path_bounds(sub, 0.0).unwrap();
        assert!(aw > 44.0 && ah > 27.0, "axis box grew: {aw}×{ah}");

        let (_, _, ow, oh) = path_bounds(sub, angle).unwrap();
        assert!(
            (ow - 40.0).abs() < 0.01 && (oh - 20.0).abs() < 0.01,
            "own box is still 40×20: {ow}×{oh}"
        );
        // And the model remembered the angle, which is what the outline reports.
        assert!((doc.paths[0].box_angle - angle).abs() < 1e-9);
    }

    #[test]
    fn bounds_cover_a_curve_not_just_its_anchors() {
        // Both anchors of this arc sit on y=80; the bulge lives entirely in the handles. Anchor-only
        // bounds called it 60×0 — a flat line — which is a lie to anyone planning around it.
        let arc = super::parse_path_d("M 10 80 C 20 10, 60 10, 70 80");
        let (_, y, w, h) = path_bounds(&arc, 0.0).unwrap();
        assert!((w - 60.0).abs() < 1e-6, "width: {w}");
        assert!(h > 60.0, "the arc has real height: {h}");
        assert!(y < 20.0, "and it reaches up: {y}");
    }

    #[test]
    fn draw_path_data_becomes_editable_anchors() {
        // The reason draw_path exists: a curve is not expressible as a bounding box, and what
        // lands has to be ordinary editable geometry, not an opaque blob.
        let curve = super::parse_path_d("M 10 80 C 20 10, 60 10, 70 80 Z");
        assert_eq!(curve.len(), 1);
        assert!(curve[0].closed, "Z closed it");
        assert!(
            curve[0].nodes.len() >= 2,
            "anchors: {}",
            curve[0].nodes.len()
        );
        assert!(
            curve[0].nodes.iter().any(|n| n.handle_out.is_some()),
            "the C carried real bezier handles through"
        );
        // Relative commands and multiple subpaths fold in the same way.
        assert_eq!(
            super::parse_path_d("M0 0 l10 0 l0 10 z M20 20 l5 0").len(),
            2
        );
        // Garbage yields nothing, so the tool can refuse rather than add an invisible path.
        assert!(super::parse_path_d("not path data").is_empty());
    }

    #[test]
    fn a_region_render_crops_without_scaling_the_whole_document() {
        // Two far-apart marks; cropping to one must show ink, and to empty space must not.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200">
  <rect x="0" y="0" width="20" height="20" fill="#000"/>
  <rect x="180" y="180" width="20" height="20" fill="#000"/>
</svg>"##;
        // Count dark pixels straight off the pixmap — no image decoder needed beyond the one
        // tiny_skia already ships for encode_png.
        let ink = |png: Vec<u8>| {
            let pm = super::tiny_skia::Pixmap::decode_png(&png).unwrap();
            pm.pixels().iter().filter(|p| p.red() < 128).count()
        };
        let corner = render_png_region(svg, 64.0, Some((0.0, 0.0, 40.0, 40.0))).unwrap();
        let middle = render_png_region(svg, 64.0, Some((80.0, 80.0, 40.0, 40.0))).unwrap();
        assert!(ink(corner) > 100, "the crop framed the mark");
        assert_eq!(ink(middle), 0, "empty space renders empty");
        // A zero-sized region is refused rather than allocating a degenerate pixmap.
        assert!(render_png_region(svg, 64.0, Some((0.0, 0.0, 0.0, 10.0))).is_err());
    }

    #[test]
    fn renders_svg_to_png() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100" fill="#3f86d4"/><circle cx="50" cy="50" r="30" fill="#ffd85e"/></svg>"##;
        let png = render_png_region(svg, 640.0, None).expect("render");
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
        let png = render_png_region(svg, 200.0, None).expect("render");
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

    /// Outlining is destructive, so `outline_text`'s addressing must never guess: an exact id or
    /// the exact words resolve, an ambiguous fragment is an error that names the candidates.
    #[test]
    fn pick_label_resolves_by_id_or_words_and_refuses_to_guess() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <text id="title" x="10" y="20">Hello</text>
  <text id="subtitle" x="10" y="40">Hello again</text>
  <text x="10" y="60">Untitled</text>
</svg>"##;
        let mut ed = nib_core::Editor::new();
        ed.load_source(svg).unwrap();
        let labels = ed.text_infos();
        assert_eq!(labels.len(), 3, "three labels: {labels:?}");

        // By id, by words, case-insensitively.
        assert_eq!(pick_label(&labels, "title").unwrap().text, "Hello");
        assert_eq!(pick_label(&labels, "SUBTITLE").unwrap().name, "subtitle");
        assert_eq!(pick_label(&labels, "Untitled").unwrap().name, "");
        // "Hello" matches one exactly even though it's a prefix of the other's words.
        assert_eq!(pick_label(&labels, "Hello").unwrap().name, "title");

        // A fragment matching two, and a miss, both fail loudly and list what's there.
        let ambiguous = pick_label(&labels, "titl").unwrap_err().to_string();
        assert!(
            ambiguous.contains("title") && ambiguous.contains("subtitle"),
            "{ambiguous}"
        );
        let missing = pick_label(&labels, "dragon").unwrap_err().to_string();
        assert!(missing.contains("no label matches"), "{missing}");
    }

    /// The server-side half of outlining: resolve a face from the system font database, shape the
    /// label with it, and apply the op. Skips on a host with no fonts (a `scratch` container), which
    /// is the same condition the tool reports to the caller.
    #[test]
    fn outlines_a_label_with_a_system_font() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><text id="title" x="10" y="60" font-size="40" fill="#111111">Hi</text></svg>"##;
        let mut ed = nib_core::Editor::new();
        ed.load_source(svg).unwrap();
        let label = ed.text_infos().first().cloned().expect("one label");
        let Some((font, index)) = crate::fonts::face_for(&label) else {
            return; // no system fonts here
        };
        let d = ed
            .text_outline_d(&label.uid, &font, index)
            .expect("glyph outlines");
        assert!(
            ed.apply(&nib_core::ops::Op::TextToPath {
                uid: label.uid.clone(),
                d,
            }),
            "the op applies"
        );
        let svg_out = ed.to_svg();
        assert!(!svg_out.contains("<text"), "label converted: {svg_out}");
        assert!(svg_out.contains("<path"), "…into a path: {svg_out}");
        // And it renders without any font loaded, which is the whole point.
        let png = render_png_region(&svg_out, 200.0, None).expect("render");
        let pm = resvg::tiny_skia::Pixmap::decode_png(&png).expect("decode");
        assert!(
            pm.data().as_chunks::<4>().0.iter().any(|p| p[0] < 235),
            "outlined glyphs paint ink"
        );
    }
}
