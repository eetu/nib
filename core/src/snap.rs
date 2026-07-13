//! The snap engine — ported from `snap/index.ts`. Anchor-to-anchor snapping, close-loop
//! detection, and grid snapping. (Smart guides / alignment snapping arrive in Phase B4.)

use serde::{Deserialize, Serialize};

use crate::model::geometry::distance;
use crate::model::types::{NodeRef, Point, SvgDocument};

/// A snappable anchor point plus the node it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SnapPoint {
    pub point: Point,
    #[serde(rename = "ref")]
    pub node_ref: NodeRef,
    /// True when this node is the first or last node of an *open* subpath — the candidates
    /// that matter for closing a loop.
    pub endpoint: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SnapResult {
    pub target: SnapPoint,
    pub distance: f64,
}

/// Gather every anchor point in the document as a snap candidate, optionally excluding one
/// node (the one being dragged).
pub fn collect_anchors(doc: &SvgDocument, exclude: Option<NodeRef>) -> Vec<SnapPoint> {
    let mut out = Vec::new();
    for (path_index, path) in doc.paths.iter().enumerate() {
        if path.deleted {
            continue;
        }
        for (subpath_index, sp) in path.subpaths.iter().enumerate() {
            let n = sp.nodes.len();
            for (node_index, node) in sp.nodes.iter().enumerate() {
                let node_ref = NodeRef {
                    path_index,
                    subpath_index,
                    node_index,
                };
                if exclude == Some(node_ref) {
                    continue;
                }
                let endpoint = !sp.closed && (node_index == 0 || node_index == n - 1);
                out.push(SnapPoint {
                    point: node.point,
                    node_ref,
                    endpoint,
                });
            }
        }
    }
    out
}

/// Nearest candidate within `threshold` (document units), or None.
pub fn find_snap(from: Point, candidates: &[SnapPoint], threshold: f64) -> Option<SnapResult> {
    let mut best: Option<SnapResult> = None;
    for c in candidates {
        let d = distance(from, c.point);
        if d <= threshold && best.is_none_or(|b| d < b.distance) {
            best = Some(SnapResult {
                target: *c,
                distance: d,
            });
        }
    }
    best
}

/// Would dragging `dragged` (an endpoint of an open subpath) onto `target` close that
/// subpath's loop? True when target is the *opposite* endpoint of the same open subpath.
pub fn is_close_loop(dragged: NodeRef, target: &SnapPoint, doc: &SvgDocument) -> bool {
    if !target.endpoint {
        return false;
    }
    if dragged.path_index != target.node_ref.path_index {
        return false;
    }
    if dragged.subpath_index != target.node_ref.subpath_index {
        return false;
    }
    let Some(sp) = doc
        .paths
        .get(dragged.path_index)
        .and_then(|p| p.subpaths.get(dragged.subpath_index))
    else {
        return false;
    };
    if sp.closed || sp.nodes.len() < 2 {
        return false;
    }
    let last = sp.nodes.len() - 1;
    let dragged_is_end = dragged.node_index == 0 || dragged.node_index == last;
    let target_is_other = target.node_ref.node_index != dragged.node_index;
    dragged_is_end && target_is_other
}

/// Snap a point to the nearest grid intersection. Uses `floor(x + 0.5)` to match JS
/// `Math.round` (round half toward +∞), so behaviour matches the TS engine exactly.
pub fn snap_to_grid(p: Point, grid: f64) -> Point {
    if grid <= 0.0 {
        return p;
    }
    Point::new(
        (p.x / grid + 0.5).floor() * grid,
        (p.y / grid + 0.5).floor() * grid,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::document::parse_svg;

    fn doc_with_open_path() -> SvgDocument {
        parse_svg(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path d="M 0 0 L 10 0 L 10 10"/></svg>"#,
        )
        .unwrap()
    }

    #[test]
    fn gathers_every_anchor_and_flags_endpoints() {
        let anchors = collect_anchors(&doc_with_open_path(), None);
        assert_eq!(anchors.len(), 3);
        assert!(anchors[0].endpoint);
        assert!(!anchors[1].endpoint);
        assert!(anchors[2].endpoint);
    }

    #[test]
    fn excludes_the_dragged_node() {
        let exclude = NodeRef {
            path_index: 0,
            subpath_index: 0,
            node_index: 2,
        };
        let anchors = collect_anchors(&doc_with_open_path(), Some(exclude));
        assert_eq!(anchors.len(), 2);
        assert!(!anchors.iter().any(|a| a.node_ref.node_index == 2));
    }

    #[test]
    fn find_snap_returns_nearest_within_threshold() {
        let anchors = collect_anchors(&doc_with_open_path(), None);
        let hit = find_snap(Point::new(0.5, 0.5), &anchors, 2.0).unwrap();
        assert_eq!(hit.target.node_ref.node_index, 0);
    }

    #[test]
    fn find_snap_returns_none_when_out_of_threshold() {
        let anchors = collect_anchors(&doc_with_open_path(), None);
        assert!(find_snap(Point::new(100.0, 100.0), &anchors, 2.0).is_none());
    }

    #[test]
    fn detects_dragging_one_endpoint_onto_the_other() {
        let doc = doc_with_open_path();
        let dragged = NodeRef {
            path_index: 0,
            subpath_index: 0,
            node_index: 2,
        };
        let anchors = collect_anchors(&doc, Some(dragged));
        let hit = find_snap(Point::new(0.4, 0.3), &anchors, 2.0).unwrap();
        assert!(is_close_loop(dragged, &hit.target, &doc));
    }

    #[test]
    fn is_false_when_snapping_to_a_non_endpoint() {
        let doc = doc_with_open_path();
        let dragged = NodeRef {
            path_index: 0,
            subpath_index: 0,
            node_index: 2,
        };
        let anchors = collect_anchors(&doc, Some(dragged));
        let hit = find_snap(Point::new(10.0, 0.0), &anchors, 2.0).unwrap();
        assert!(!is_close_loop(dragged, &hit.target, &doc));
    }

    #[test]
    fn snap_to_grid_rounds_to_nearest_intersection() {
        assert_eq!(
            snap_to_grid(Point::new(11.0, 4.0), 10.0),
            Point::new(10.0, 0.0)
        );
        assert_eq!(
            snap_to_grid(Point::new(16.0, 15.0), 10.0),
            Point::new(20.0, 20.0)
        );
    }
}
