//! Vector + bezier geometry — ported from `model/geometry.ts`.

use serde::{Deserialize, Serialize};

use super::types::{PathNode, Point, Subpath};

pub fn add(a: Point, b: Point) -> Point {
    Point::new(a.x + b.x, a.y + b.y)
}

pub fn sub(a: Point, b: Point) -> Point {
    Point::new(a.x - b.x, a.y - b.y)
}

pub fn scale(a: Point, k: f64) -> Point {
    Point::new(a.x * k, a.y * k)
}

pub fn lerp(a: Point, b: Point, t: f64) -> Point {
    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

pub fn distance(a: Point, b: Point) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

pub fn length(v: Point) -> f64 {
    v.x.hypot(v.y)
}

/// Unit vector in the direction of v (zero vector maps to {0,0}).
pub fn normalize(v: Point) -> Point {
    let l = length(v);
    if l < 1e-9 {
        Point::new(0.0, 0.0)
    } else {
        Point::new(v.x / l, v.y / l)
    }
}

/// An axis-aligned bounding box in document units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    #[serde(rename = "minX")]
    pub min_x: f64,
    #[serde(rename = "minY")]
    pub min_y: f64,
    #[serde(rename = "maxX")]
    pub max_x: f64,
    #[serde(rename = "maxY")]
    pub max_y: f64,
}

/// Bounding box (doc units) of subpaths, from nodes + handles — a valid bound for a
/// selection box (bezier curves stay within their control points).
pub fn subpaths_bounds(subpaths: &[Subpath]) -> Option<Bounds> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut any = false;
    for sp in subpaths {
        for n in &sp.nodes {
            for p in [Some(n.point), n.handle_in, n.handle_out]
                .into_iter()
                .flatten()
            {
                any = true;
                min_x = min_x.min(p.x);
                min_y = min_y.min(p.y);
                max_x = max_x.max(p.x);
                max_y = max_y.max(p.y);
            }
        }
    }
    if any {
        Some(Bounds {
            min_x,
            min_y,
            max_x,
            max_y,
        })
    } else {
        None
    }
}

/// Rotate subpaths (each node's point + both handles) about the pivot `(cx, cy)` by `radians`,
/// clockwise in the y-down document space (matching SVG's `rotate()` and the client transform box).
/// Returns fresh subpaths; the input is untouched. The single rotation kernel `RotatePath` funnels
/// through — the transform box, a numeric field, and the MCP `rotate` tool all land here.
pub fn rotate_subpaths(subpaths: &[Subpath], cx: f64, cy: f64, radians: f64) -> Vec<Subpath> {
    let (sin, cos) = radians.sin_cos();
    let at = |p: Point| -> Point {
        let dx = p.x - cx;
        let dy = p.y - cy;
        Point::new(cx + dx * cos - dy * sin, cy + dx * sin + dy * cos)
    };
    subpaths
        .iter()
        .map(|sp| Subpath {
            closed: sp.closed,
            nodes: sp
                .nodes
                .iter()
                .map(|n| PathNode {
                    point: at(n.point),
                    handle_in: n.handle_in.map(at),
                    handle_out: n.handle_out.map(at),
                    node_type: n.node_type,
                })
                .collect(),
        })
        .collect()
}

/// Are the incoming/outgoing handles of a node collinear through the point (i.e. the node
/// reads as smooth)? Tolerant of handle length.
pub fn handles_collinear(handle_in: Point, point: Point, handle_out: Point, eps_deg: f64) -> bool {
    let a = sub(point, handle_in); // direction into the point
    let b = sub(handle_out, point); // direction out of the point
    let la = length(a);
    let lb = length(b);
    if la < 1e-6 || lb < 1e-6 {
        return false;
    }
    let cross = a.x * b.y - a.y * b.x;
    let sin = cross.abs() / (la * lb);
    sin <= (eps_deg * std::f64::consts::PI / 180.0).sin()
}

/// The two sub-curves' control points from splitting a cubic (p0..p3) at parameter t.
pub struct CubicSplit {
    pub left: [Point; 4],
    pub right: [Point; 4],
    pub point: Point,
}

/// Split a cubic bezier (p0..p3) at parameter t via de Casteljau, returning the two
/// sub-curves' control points. Used to insert a node on a segment without changing shape.
pub fn split_cubic(p0: Point, p1: Point, p2: Point, p3: Point, t: f64) -> CubicSplit {
    let a = lerp(p0, p1, t);
    let b = lerp(p1, p2, t);
    let c = lerp(p2, p3, t);
    let d = lerp(a, b, t);
    let e = lerp(b, c, t);
    let f = lerp(d, e, t); // the point on the curve at t
    CubicSplit {
        left: [p0, a, d, f],
        right: [f, e, c, p3],
        point: f,
    }
}

/// Evaluate a cubic bezier at t.
pub fn cubic_at(p0: Point, p1: Point, p2: Point, p3: Point, t: f64) -> Point {
    let u = 1.0 - t;
    let w0 = u * u * u;
    let w1 = 3.0 * u * u * t;
    let w2 = 3.0 * u * t * t;
    let w3 = t * t * t;
    Point::new(
        w0 * p0.x + w1 * p1.x + w2 * p2.x + w3 * p3.x,
        w0 * p0.y + w1 * p1.y + w2 * p2.y + w3 * p3.y,
    )
}
