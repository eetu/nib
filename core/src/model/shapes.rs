//! Editable shape builders — ported from `model/shapes.ts`. Shapes are ordinary anchor-node
//! paths (drag their nodes to reshape), not native SVG primitives.

use super::types::{NodeType, PathNode, Point, Subpath};

/// The magic constant for approximating a quarter circle with a cubic bezier.
const KAPPA: f64 = 0.5522847498307936;

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
