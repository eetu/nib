//! The `d` ↔ anchor-node walker + serializer — ported from `model/path.ts`.
//!
//! Parsing normalizes any path to absolute cubic anchor nodes. Where the TS used
//! `svg-pathdata`'s `.toAbs().normalizeST().qtToC().aToC()`, we use `kurbo::BezPath::from_svg`
//! (already absolute; arcs → cubics, H/V → lines, S/T reflected) and elevate its `QuadTo`
//! elements to cubics here — the one normalization kurbo leaves to us.

use kurbo::{BezPath, PathEl};

use super::geometry::{cubic_at, distance, handles_collinear, lerp, split_cubic};
use super::types::{NodeType, PathNode, Point, Subpath};

const EPS: f64 = 1e-4;

/// Perpendicular distance from `p` to the line through `a`,`b` (falls back to |p-a| if a==b).
fn perp_distance(p: Point, a: Point, b: Point) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-12 {
        return distance(p, a);
    }
    ((p.x - a.x) * dy - (p.y - a.y) * dx).abs() / len
}

/// Ramer–Douglas–Peucker: indices of `pts` to keep so the polyline stays within `eps`.
fn rdp_keep(pts: &[Point], eps: f64) -> Vec<usize> {
    let n = pts.len();
    if n <= 2 {
        return (0..n).collect();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((first, last)) = stack.pop() {
        let mut max_d = 0.0;
        let mut idx = first;
        for i in (first + 1)..last {
            let d = perp_distance(pts[i], pts[first], pts[last]);
            if d > max_d {
                max_d = d;
                idx = i;
            }
        }
        if max_d > eps {
            keep[idx] = true;
            stack.push((first, idx));
            stack.push((idx, last));
        }
    }
    let mut kept: Vec<usize> = (0..n).filter(|&i| keep[i]).collect();
    kept.sort_unstable();
    kept
}

/// Reduce each subpath's node count with RDP over its anchor points (survivors keep their
/// handles), within `eps` document units. Great for thinning the dense polylines boolean ops
/// produce; subpaths of ≤ 2 nodes are left untouched.
pub fn simplify_subpaths(subpaths: &[Subpath], eps: f64) -> Vec<Subpath> {
    subpaths
        .iter()
        .map(|sp| {
            if sp.nodes.len() <= 2 {
                return sp.clone();
            }
            let pts: Vec<Point> = sp.nodes.iter().map(|n| n.point).collect();
            let keep = rdp_keep(&pts, eps);
            Subpath {
                nodes: keep.iter().map(|&i| sp.nodes[i]).collect(),
                closed: sp.closed,
            }
        })
        .collect()
}

/// Parse a path `d` string into editable subpaths of cubic anchor nodes.
pub fn parse_path_d(d: &str) -> Vec<Subpath> {
    let Ok(bez) = BezPath::from_svg(d) else {
        return Vec::new();
    };
    let mut subpaths: Vec<Subpath> = Vec::new();

    for el in bez.iter() {
        match el {
            PathEl::MoveTo(p) => {
                subpaths.push(Subpath {
                    nodes: vec![PathNode::corner(Point::new(p.x, p.y))],
                    closed: false,
                });
            }
            PathEl::LineTo(p) => {
                if let Some(sp) = subpaths.last_mut() {
                    sp.nodes.push(PathNode::corner(Point::new(p.x, p.y)));
                }
            }
            PathEl::QuadTo(c, p) => {
                if let Some(sp) = subpaths.last_mut() {
                    let p0 = sp
                        .nodes
                        .last()
                        .map(|n| n.point)
                        .unwrap_or(Point::new(c.x, c.y));
                    let end = Point::new(p.x, p.y);
                    let ctrl = Point::new(c.x, c.y);
                    // Elevate quadratic → cubic (matches svg-pathdata's qtToC).
                    let c1 = Point::new(
                        p0.x + 2.0 / 3.0 * (ctrl.x - p0.x),
                        p0.y + 2.0 / 3.0 * (ctrl.y - p0.y),
                    );
                    let c2 = Point::new(
                        end.x + 2.0 / 3.0 * (ctrl.x - end.x),
                        end.y + 2.0 / 3.0 * (ctrl.y - end.y),
                    );
                    if let Some(prev) = sp.nodes.last_mut() {
                        prev.handle_out = Some(c1);
                    }
                    sp.nodes.push(PathNode {
                        point: end,
                        handle_in: Some(c2),
                        handle_out: None,
                        node_type: NodeType::Corner,
                    });
                }
            }
            PathEl::CurveTo(c1, c2, p) => {
                if let Some(sp) = subpaths.last_mut() {
                    if let Some(prev) = sp.nodes.last_mut() {
                        prev.handle_out = Some(Point::new(c1.x, c1.y));
                    }
                    sp.nodes.push(PathNode {
                        point: Point::new(p.x, p.y),
                        handle_in: Some(Point::new(c2.x, c2.y)),
                        handle_out: None,
                        node_type: NodeType::Corner,
                    });
                }
            }
            PathEl::ClosePath => {
                if let Some(sp) = subpaths.last_mut() {
                    sp.closed = true;
                }
            }
        }
    }

    for sp in subpaths.iter_mut() {
        fold_closing_node(sp);
    }
    for sp in subpaths.iter_mut() {
        infer_node_types(sp);
    }
    subpaths
}

/// When a closed subpath's last node lands on its first node (a curve that ended exactly at
/// the start before Z), fold that trailing node's incoming handle onto the first node and
/// drop it — leaving a clean cyclic model where the closing segment is last→first.
fn fold_closing_node(sp: &mut Subpath) {
    if !sp.closed || sp.nodes.len() < 2 {
        return;
    }
    let first = sp.nodes[0].point;
    let last_idx = sp.nodes.len() - 1;
    let last = sp.nodes[last_idx].point;
    if distance(first, last) <= EPS {
        if let Some(h) = sp.nodes[last_idx].handle_in {
            sp.nodes[0].handle_in = Some(h);
        }
        sp.nodes.pop();
    }
}

/// Mark a subpath closed, folding away a last node that coincides with the first.
pub fn close_subpath(sp: &mut Subpath) {
    if sp.nodes.len() < 2 {
        return;
    }
    sp.closed = true;
    fold_closing_node(sp);
}

/// The same subpath drawn in the opposite direction: node order reversed and each node's
/// in/out handles swapped (they trade roles when the direction flips).
pub fn reversed_subpath(sp: &Subpath) -> Subpath {
    let nodes = sp
        .nodes
        .iter()
        .rev()
        .map(|n| PathNode {
            point: n.point,
            handle_in: n.handle_out,
            handle_out: n.handle_in,
            node_type: n.node_type,
        })
        .collect();
    Subpath {
        closed: sp.closed,
        nodes,
    }
}

fn infer_node_types(sp: &mut Subpath) {
    for node in sp.nodes.iter_mut() {
        if let (Some(hi), Some(ho)) = (node.handle_in, node.handle_out) {
            node.node_type = if handles_collinear(hi, node.point, ho, 3.0) {
                NodeType::Smooth
            } else {
                NodeType::Corner
            };
        }
    }
}

/// Format a number like TS `String(Number(v.toFixed(precision)))`: fixed to `precision`
/// decimals, then trailing zeros (and a bare trailing dot) stripped, and -0 collapsed to 0.
fn fmt(v: f64, precision: usize) -> String {
    let s = format!("{v:.precision$}");
    let trimmed = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s.as_str()
    };
    if trimmed.is_empty() || trimmed == "-0" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

fn pt(p: Point, precision: usize) -> String {
    format!("{} {}", fmt(p.x, precision), fmt(p.y, precision))
}

/// A segment a→b: a line iff both adjoining handles are absent, else a cubic (a missing
/// control defaults to its own anchor, i.e. a "half-straight").
fn segment(a: &PathNode, b: &PathNode, precision: usize) -> String {
    if a.handle_out.is_none() && b.handle_in.is_none() {
        return format!("L {}", pt(b.point, precision));
    }
    let c1 = a.handle_out.unwrap_or(a.point);
    let c2 = b.handle_in.unwrap_or(b.point);
    format!(
        "C {} {} {}",
        pt(c1, precision),
        pt(c2, precision),
        pt(b.point, precision)
    )
}

/// The four cubic control points of the segment leaving node `i` (wrapping to node 0 for
/// the closing segment of a closed subpath).
pub fn segment_control_points(sp: &Subpath, i: usize) -> [Point; 4] {
    let n = sp.nodes.len();
    let a = &sp.nodes[i];
    let b = &sp.nodes[(i + 1) % n];
    [
        a.point,
        a.handle_out.unwrap_or(a.point),
        b.handle_in.unwrap_or(b.point),
        b.point,
    ]
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentHit {
    pub segment_index: usize,
    pub t: f64,
    pub point: Point,
    pub distance: f64,
}

/// Nearest point on a subpath's outline to `target`, using the TS default of 32 samples.
pub fn nearest_on_subpath(sp: &Subpath, target: Point) -> Option<SegmentHit> {
    nearest_on_subpath_n(sp, target, 32)
}

/// Nearest point on a subpath's outline to `target`. A coarse per-segment sample picks the
/// winning segment + rough parameter, then a local ternary refinement pins the true nearest
/// point. `segment_index` is the node the segment leaves.
pub fn nearest_on_subpath_n(sp: &Subpath, target: Point, samples: usize) -> Option<SegmentHit> {
    let n = sp.nodes.len();
    if n < 2 {
        return None;
    }
    let last_seg = if sp.closed { n - 1 } else { n - 2 };
    let mut best_seg: Option<usize> = None;
    let mut best_t = 0.0;
    let mut best_d = f64::INFINITY;
    for i in 0..=last_seg {
        let [p0, p1, p2, p3] = segment_control_points(sp, i);
        for s in 0..=samples {
            let t = s as f64 / samples as f64;
            let d = distance(cubic_at(p0, p1, p2, p3, t), target);
            if d < best_d {
                best_d = d;
                best_seg = Some(i);
                best_t = t;
            }
        }
    }
    let bi = best_seg?;

    let [q0, q1, q2, q3] = segment_control_points(sp, bi);
    let mut lo = (best_t - 1.0 / samples as f64).max(0.0);
    let mut hi = (best_t + 1.0 / samples as f64).min(1.0);
    for _ in 0..24 {
        let m1 = lo + (hi - lo) / 3.0;
        let m2 = hi - (hi - lo) / 3.0;
        let d1 = distance(cubic_at(q0, q1, q2, q3, m1), target);
        let d2 = distance(cubic_at(q0, q1, q2, q3, m2), target);
        if d1 < d2 {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    let t = (lo + hi) / 2.0;
    let point = cubic_at(q0, q1, q2, q3, t);
    Some(SegmentHit {
        segment_index: bi,
        t,
        point,
        distance: distance(point, target),
    })
}

/// Insert a node on the segment leaving node `i` at parameter `t`, preserving the curve's
/// shape (de Casteljau split; a straight segment stays straight). Returns the new index.
pub fn insert_node_at(sp: &mut Subpath, i: usize, t: f64) -> usize {
    let n = sp.nodes.len();
    let b_idx = (i + 1) % n;
    let a_straight = sp.nodes[i].handle_out.is_none();
    let b_straight = sp.nodes[b_idx].handle_in.is_none();
    if a_straight && b_straight {
        let point = lerp(sp.nodes[i].point, sp.nodes[b_idx].point, t);
        sp.nodes.insert(i + 1, PathNode::corner(point));
        return i + 1;
    }
    let [p0, p1, p2, p3] = segment_control_points(sp, i);
    let split = split_cubic(p0, p1, p2, p3, t);
    if sp.nodes[i].handle_out.is_some() {
        sp.nodes[i].handle_out = Some(split.left[1]);
    }
    if sp.nodes[b_idx].handle_in.is_some() {
        sp.nodes[b_idx].handle_in = Some(split.right[2]);
    }
    let new_node = PathNode {
        point: split.point,
        handle_in: Some(split.left[2]),
        handle_out: Some(split.right[1]),
        node_type: NodeType::Smooth,
    };
    sp.nodes.insert(i + 1, new_node);
    i + 1
}

/// Serialize subpaths to a compact absolute `d` string at the TS default precision (3).
pub fn path_to_d(subpaths: &[Subpath]) -> String {
    path_to_d_prec(subpaths, 3)
}

/// Serialize subpaths to a compact absolute `d` string at the given coordinate precision.
pub fn path_to_d_prec(subpaths: &[Subpath], precision: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    for sp in subpaths {
        if sp.nodes.is_empty() {
            continue;
        }
        let n = &sp.nodes;
        parts.push(format!("M {}", pt(n[0].point, precision)));
        for i in 1..n.len() {
            parts.push(segment(&n[i - 1], &n[i], precision));
        }
        if sp.closed && n.len() >= 2 {
            let last = &n[n.len() - 1];
            let first = &n[0];
            // Emit an explicit closing curve only when the seam curves; a straight close is
            // just Z (implicit line back to the start).
            if last.handle_out.is_some() || first.handle_in.is_some() {
                parts.push(segment(last, first, precision));
            }
            parts.push("Z".to_string());
        }
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn simplify_drops_collinear_nodes_but_keeps_corners() {
        let line = Subpath {
            nodes: [0.0, 1.0, 2.0, 3.0]
                .iter()
                .map(|&x| PathNode::corner(Point::new(x, 0.0)))
                .collect(),
            closed: false,
        };
        assert_eq!(simplify_subpaths(&[line], 0.1)[0].nodes.len(), 2);

        let bent = Subpath {
            nodes: vec![
                PathNode::corner(Point::new(0.0, 0.0)),
                PathNode::corner(Point::new(1.0, 0.0)),
                PathNode::corner(Point::new(1.0, 1.0)), // a real corner
                PathNode::corner(Point::new(2.0, 1.0)),
            ],
            closed: false,
        };
        assert!(simplify_subpaths(&[bent], 0.1)[0].nodes.len() >= 3);
    }

    #[test]
    fn parses_a_line_based_open_path_into_anchor_nodes() {
        let sp = parse_path_d("M 0 0 L 10 0 L 10 10");
        assert_eq!(sp.len(), 1);
        assert!(!sp[0].closed);
        let pts: Vec<Point> = sp[0].nodes.iter().map(|n| n.point).collect();
        assert_eq!(
            pts,
            vec![
                Point::new(0.0, 0.0),
                Point::new(10.0, 0.0),
                Point::new(10.0, 10.0)
            ]
        );
        assert!(
            sp[0]
                .nodes
                .iter()
                .all(|n| n.handle_in.is_none() && n.handle_out.is_none())
        );
    }

    #[test]
    fn captures_cubic_control_handles_on_the_adjoining_nodes() {
        let sp = parse_path_d("M 0 0 C 0 10 10 10 10 0");
        assert_eq!(sp[0].nodes.len(), 2);
        assert_eq!(sp[0].nodes[0].handle_out, Some(Point::new(0.0, 10.0)));
        assert_eq!(sp[0].nodes[1].handle_in, Some(Point::new(10.0, 10.0)));
    }

    #[test]
    fn normalizes_relative_and_shorthand_commands_to_absolute_cubics() {
        let sp = parse_path_d("M 5 5 h 10 v 10 z");
        assert!(sp[0].closed);
        let pts: Vec<Point> = sp[0].nodes.iter().map(|n| n.point).collect();
        assert_eq!(
            pts,
            vec![
                Point::new(5.0, 5.0),
                Point::new(15.0, 5.0),
                Point::new(15.0, 15.0)
            ]
        );
    }

    #[test]
    fn folds_a_curve_that_closes_exactly_on_the_start_point() {
        let sp = parse_path_d("M 0 0 C 5 0 10 5 10 10 C 5 10 0 5 0 0 Z");
        assert!(sp[0].closed);
        assert_eq!(sp[0].nodes.len(), 2);
        assert_eq!(sp[0].nodes[0].handle_in, Some(Point::new(0.0, 5.0)));
    }

    #[test]
    fn marks_collinear_handle_nodes_as_smooth() {
        let sp = parse_path_d("M 0 5 C 0 5 5 10 5 5 C 5 0 10 5 10 5");
        assert_eq!(sp[0].nodes[1].node_type, NodeType::Smooth);
    }

    #[test]
    fn converts_arcs_to_cubics_rather_than_dropping_them() {
        let sp = parse_path_d("M 0 0 A 5 5 0 0 1 10 0");
        assert!(sp[0].nodes.len() > 1);
        assert!(
            sp[0]
                .nodes
                .iter()
                .any(|n| n.handle_in.is_some() || n.handle_out.is_some())
        );
    }

    #[test]
    fn round_trips_an_open_line_path() {
        assert_eq!(
            path_to_d(&parse_path_d("M 0 0 L 10 0 L 10 10")),
            "M 0 0 L 10 0 L 10 10"
        );
    }

    #[test]
    fn round_trips_a_cubic() {
        assert_eq!(
            path_to_d(&parse_path_d("M 0 0 C 0 10 10 10 10 0")),
            "M 0 0 C 0 10 10 10 10 0"
        );
    }

    #[test]
    fn emits_z_for_a_straight_closed_subpath() {
        assert_eq!(
            path_to_d(&parse_path_d("M 0 0 L 10 0 L 10 10 Z")),
            "M 0 0 L 10 0 L 10 10 Z"
        );
    }

    #[test]
    fn rounds_coordinates_to_the_requested_precision() {
        assert_eq!(
            path_to_d_prec(&parse_path_d("M 0.123456 0 L 10 0"), 2),
            "M 0.12 0 L 10 0"
        );
    }

    #[test]
    fn reverses_node_order_and_swaps_handles() {
        let sp = Subpath {
            closed: false,
            nodes: vec![
                PathNode {
                    point: Point::new(0.0, 0.0),
                    handle_in: None,
                    handle_out: Some(Point::new(1.0, 1.0)),
                    node_type: NodeType::Corner,
                },
                PathNode {
                    point: Point::new(10.0, 0.0),
                    handle_in: Some(Point::new(8.0, 0.0)),
                    handle_out: Some(Point::new(12.0, 0.0)),
                    node_type: NodeType::Smooth,
                },
                PathNode {
                    point: Point::new(20.0, 0.0),
                    handle_in: Some(Point::new(19.0, 1.0)),
                    handle_out: None,
                    node_type: NodeType::Corner,
                },
            ],
        };
        let r = reversed_subpath(&sp);
        assert!(!r.closed);
        let pts: Vec<Point> = r.nodes.iter().map(|n| n.point).collect();
        assert_eq!(
            pts,
            vec![
                Point::new(20.0, 0.0),
                Point::new(10.0, 0.0),
                Point::new(0.0, 0.0)
            ]
        );
        assert_eq!(r.nodes[0].handle_out, Some(Point::new(19.0, 1.0)));
        assert_eq!(r.nodes[0].handle_in, None);
        assert_eq!(r.nodes[1].handle_in, Some(Point::new(12.0, 0.0)));
        assert_eq!(r.nodes[1].handle_out, Some(Point::new(8.0, 0.0)));
        assert_eq!(r.nodes[2].handle_in, Some(Point::new(1.0, 1.0)));
        assert_eq!(r.nodes[2].handle_out, None);
    }

    #[test]
    fn re_serializes_to_the_same_geometry_drawn_backwards() {
        let parsed = parse_path_d("M 0 0 C 0 10 10 10 10 0");
        assert_eq!(
            path_to_d(&[reversed_subpath(&parsed[0])]),
            "M 10 0 C 10 10 0 10 0 0"
        );
    }

    #[test]
    fn close_subpath_closes_and_emits_trailing_z() {
        let mut sp = parse_path_d("M 0 0 L 10 0 L 10 10").remove(0);
        close_subpath(&mut sp);
        assert!(sp.closed);
        assert_eq!(path_to_d(&[sp]), "M 0 0 L 10 0 L 10 10 Z");
    }

    #[test]
    fn close_subpath_folds_a_coincident_endpoint() {
        let mut sp = parse_path_d("M 0 0 L 10 0 L 0 0").remove(0);
        assert_eq!(sp.nodes.len(), 3);
        close_subpath(&mut sp);
        assert_eq!(sp.nodes.len(), 2);
        assert!(sp.closed);
    }

    #[test]
    fn insert_node_at_midpoint_of_straight_segment() {
        let mut sp = parse_path_d("M 0 0 L 10 0").remove(0);
        let idx = insert_node_at(&mut sp, 0, 0.5);
        assert_eq!(idx, 1);
        assert_eq!(sp.nodes.len(), 3);
        assert_eq!(sp.nodes[1].point, Point::new(5.0, 0.0));
        assert_eq!(sp.nodes[1].handle_in, None);
    }

    #[test]
    fn insert_node_at_splits_a_cubic_preserving_shape() {
        let mut sp = parse_path_d("M 0 0 C 0 10 10 10 10 0").remove(0);
        let [p0, p1, p2, p3] = segment_control_points(&sp, 0);
        let mid = cubic_at(p0, p1, p2, p3, 0.5);
        insert_node_at(&mut sp, 0, 0.5);
        assert_eq!(sp.nodes.len(), 3);
        assert!(close(sp.nodes[1].point.x, mid.x));
        assert!(close(sp.nodes[1].point.y, mid.y));
        assert!(sp.nodes[1].handle_in.is_some());
        assert!(sp.nodes[1].handle_out.is_some());
    }

    #[test]
    fn nearest_on_subpath_finds_closest_segment() {
        let sp = parse_path_d("M 0 0 L 10 0 L 10 10").remove(0);
        let hit = nearest_on_subpath(&sp, Point::new(5.0, 1.0)).unwrap();
        assert_eq!(hit.segment_index, 0);
        assert!(close(hit.point.y, 0.0));
    }

    #[test]
    fn nearest_on_subpath_considers_the_closing_segment() {
        let sp = parse_path_d("M 0 0 L 10 0 L 10 10 Z").remove(0);
        let hit = nearest_on_subpath(&sp, Point::new(5.0, 5.0)).unwrap();
        assert_eq!(hit.segment_index, 2);
    }
}
