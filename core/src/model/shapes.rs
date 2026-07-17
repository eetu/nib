//! Editable shape builders — ported from `model/shapes.ts`. Shapes are ordinary anchor-node
//! paths (drag their nodes to reshape), not native SVG primitives.

use std::f64::consts::PI;

use super::types::{NodeType, PathNode, Point, Subpath};

/// The magic constant for approximating a quarter circle with a cubic bezier.
const KAPPA: f64 = 0.5522847498307936;

/// Upper bound on polygon sides / star points. The op surface is untrusted (UI + MCP), and an
/// unbounded count would allocate a huge node Vec (OOM) — `star_nodes`' `2 * points` would also
/// overflow `u32`. No real shape needs more than this; clamp instead of trusting the input.
const MAX_SHAPE_POINTS: u32 = 1024;

fn corner(x: f64, y: f64) -> PathNode {
    PathNode::corner(Point::new(x, y))
}

/// Nodes of an axis-aligned rectangle (coords normalized so any drag direction works). `rx`/`ry`
/// are the corner radii: `0` (the default) gives a sharp 4-corner rect; a positive radius gives a
/// rounded rect — 8 smooth nodes with quarter-ellipse bezier corners (radii clamped to half the
/// side). `ry <= 0` mirrors `rx`, so a single radius rounds evenly. Closed.
pub fn rect_nodes(x0: f64, y0: f64, x1: f64, y1: f64, rx: f64, ry: f64) -> Vec<PathNode> {
    let (ax, bx) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
    let (ay, by) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
    let crx = rx.max(0.0).min((bx - ax) / 2.0);
    let cry = if ry > 0.0 { ry } else { rx }.max(0.0).min((by - ay) / 2.0);
    if crx < 1e-6 || cry < 1e-6 {
        return vec![
            corner(ax, ay),
            corner(bx, ay),
            corner(bx, by),
            corner(ax, by),
        ];
    }
    let (kx, ky) = (KAPPA * crx, KAPPA * cry);
    // Clockwise from the top edge's left end. Each node touches one straight edge (no handle that
    // side) and one corner arc (a bezier handle toward the corner) — tangent-continuous, so smooth.
    let smooth = |x: f64, y: f64, hin: Option<(f64, f64)>, hout: Option<(f64, f64)>| PathNode {
        point: Point::new(x, y),
        handle_in: hin.map(|(hx, hy)| Point::new(hx, hy)),
        handle_out: hout.map(|(hx, hy)| Point::new(hx, hy)),
        node_type: NodeType::Smooth,
    };
    vec![
        smooth(ax + crx, ay, Some((ax + crx - kx, ay)), None), // top-left → top edge
        smooth(bx - crx, ay, None, Some((bx - crx + kx, ay))), // top edge → top-right arc
        smooth(bx, ay + cry, Some((bx, ay + cry - ky)), None), // right edge
        smooth(bx, by - cry, None, Some((bx, by - cry + ky))), // right edge → bottom-right arc
        smooth(bx - crx, by, Some((bx - crx + kx, by)), None), // bottom edge
        smooth(ax + crx, by, None, Some((ax + crx - kx, by))), // bottom edge → bottom-left arc
        smooth(ax, by - cry, Some((ax, by - cry + ky)), None), // left edge
        smooth(ax, ay + cry, None, Some((ax, ay + cry - ky))), // left edge → top-left arc
    ]
}

/// Two corner nodes of a straight line segment (open subpath).
pub fn line_nodes(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<PathNode> {
    vec![corner(x0, y0), corner(x1, y1)]
}

/// `sides` corner nodes of a regular polygon on a circle of radius `r`, `rotation` radians
/// from the +x axis (callers pass -PI/2 to put a vertex up).
pub fn polygon_nodes(cx: f64, cy: f64, r: f64, sides: u32, rotation: f64) -> Vec<PathNode> {
    let n = sides.clamp(3, MAX_SHAPE_POINTS);
    (0..n)
        .map(|i| {
            let a = rotation + 2.0 * PI * (i as f64) / (n as f64);
            corner(cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// `2 * points` corner nodes of a star, alternating `outer`/`inner` radius.
pub fn star_nodes(
    cx: f64,
    cy: f64,
    outer: f64,
    inner: f64,
    points: u32,
    rotation: f64,
) -> Vec<PathNode> {
    let n = points.clamp(2, MAX_SHAPE_POINTS);
    (0..2 * n)
        .map(|i| {
            let r = if i % 2 == 0 { outer } else { inner };
            let a = rotation + PI * (i as f64) / (n as f64);
            corner(cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// Four smooth cubic-bezier nodes (E, S, W, N) approximating an ellipse centred at (cx, cy)
/// with radii rx/ry.
pub fn ellipse_nodes(cx: f64, cy: f64, rx: f64, ry: f64) -> Vec<PathNode> {
    let kx = KAPPA * rx;
    let ky = KAPPA * ry;
    vec![
        PathNode {
            point: Point::new(cx + rx, cy),
            handle_in: Some(Point::new(cx + rx, cy - ky)),
            handle_out: Some(Point::new(cx + rx, cy + ky)),
            node_type: NodeType::Smooth,
        },
        PathNode {
            point: Point::new(cx, cy + ry),
            handle_in: Some(Point::new(cx + kx, cy + ry)),
            handle_out: Some(Point::new(cx - kx, cy + ry)),
            node_type: NodeType::Smooth,
        },
        PathNode {
            point: Point::new(cx - rx, cy),
            handle_in: Some(Point::new(cx - rx, cy + ky)),
            handle_out: Some(Point::new(cx - rx, cy - ky)),
            node_type: NodeType::Smooth,
        },
        PathNode {
            point: Point::new(cx, cy - ry),
            handle_in: Some(Point::new(cx - kx, cy - ry)),
            handle_out: Some(Point::new(cx + kx, cy - ry)),
            node_type: NodeType::Smooth,
        },
    ]
}

pub fn ellipse_subpath(cx: f64, cy: f64, rx: f64, ry: f64) -> Subpath {
    Subpath {
        nodes: ellipse_nodes(cx, cy, rx, ry),
        closed: true,
    }
}
