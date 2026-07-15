//! Boolean path operations (union / intersect / subtract / exclude) over the `i_overlay`
//! polygon kernel. Curves are flattened to polylines first, so the result is a corner-node
//! path — a deliberate V1 fidelity tradeoff; curve-accurate booleans are a follow-up.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;

use super::geometry::cubic_at;
use super::path::segment_control_points;
use super::types::{PathElement, PathNode, Point, Subpath};

// Samples per cubic segment when flattening to a polygon contour.
const FLATTEN_STEPS: usize = 24;

fn subpath_to_contour(sp: &Subpath) -> Vec<[f64; 2]> {
    let n = sp.nodes.len();
    if n < 2 {
        return sp.nodes.iter().map(|nd| [nd.point.x, nd.point.y]).collect();
    }
    let segs = if sp.closed { n } else { n - 1 };
    let mut pts = Vec::with_capacity(segs * FLATTEN_STEPS);
    for i in 0..segs {
        let [p0, p1, p2, p3] = segment_control_points(sp, i);
        // Sample [0, STEPS) per segment — t=1 is the next segment's t=0, so skipping it
        // avoids duplicate vertices; the contour closes implicitly.
        for s in 0..FLATTEN_STEPS {
            let t = s as f64 / FLATTEN_STEPS as f64;
            let p = cubic_at(p0, p1, p2, p3, t);
            pts.push([p.x, p.y]);
        }
    }
    pts
}

fn subpaths_to_contours(subpaths: &[Subpath]) -> Vec<Vec<[f64; 2]>> {
    subpaths
        .iter()
        .map(subpath_to_contour)
        .filter(|c| c.len() >= 3)
        .collect()
}

fn path_to_contours(p: &PathElement) -> Vec<Vec<[f64; 2]>> {
    subpaths_to_contours(&p.subpaths)
}

/// Convert an i_overlay result (shapes → contours → points) into closed corner-node subpaths.
fn shapes_to_subpaths(shapes: &[Vec<Vec<[f64; 2]>>]) -> Vec<Subpath> {
    shapes
        .iter()
        .flat_map(|shape| shape.iter())
        .filter(|contour| contour.len() >= 3)
        .map(|contour| Subpath {
            nodes: contour
                .iter()
                .map(|pt| PathNode::corner(Point::new(pt[0], pt[1])))
                .collect(),
            closed: true,
        })
        .collect()
}

fn rule_for(op: &str) -> Option<OverlayRule> {
    match op {
        "union" => Some(OverlayRule::Union),
        "intersect" => Some(OverlayRule::Intersect),
        "subtract" => Some(OverlayRule::Difference),
        "exclude" => Some(OverlayRule::Xor),
        _ => None,
    }
}

/// Fold the boolean `op` across `paths` (≥ 2, folded in order — for `subtract`, the first is
/// the subject and the rest are removed). Returns the result as flattened, closed corner-node
/// subpaths (each output contour → one subpath), or `None` if the op is unknown / empty.
pub fn boolean(op: &str, paths: &[&PathElement]) -> Option<Vec<Subpath>> {
    let rule = rule_for(op)?;
    if paths.len() < 2 {
        return None;
    }
    let c0 = path_to_contours(paths[0]);
    let c1 = path_to_contours(paths[1]);
    if c0.is_empty() || c1.is_empty() {
        return None;
    }
    let mut acc = c0.overlay(&c1, rule, FillRule::NonZero);
    for p in &paths[2..] {
        let clip = path_to_contours(p);
        if clip.is_empty() {
            continue;
        }
        acc = acc.overlay(&clip, rule, FillRule::NonZero);
    }
    let subpaths = shapes_to_subpaths(&acc);
    (!subpaths.is_empty()).then_some(subpaths)
}

/// Offset a path's outline by `d` document units (outward if positive, inward if negative).
/// Built from the kernels we already have: a stroke band of width `2|d|` unioned onto the fill
/// grows it by `d`; subtracting the band from the fill shrinks it by `d`. Flattened result.
pub fn offset_path(subpaths: &[Subpath], d: f64) -> Option<Vec<Subpath>> {
    if d.abs() < 1e-9 {
        return Some(subpaths.to_vec());
    }
    let fill = subpaths_to_contours(subpaths);
    if fill.is_empty() {
        return None;
    }
    let band_subpaths = crate::model::path::outline_stroke(subpaths, 2.0 * d.abs(), 0.25);
    let band = subpaths_to_contours(&band_subpaths);
    if band.is_empty() {
        return None;
    }
    let rule = if d > 0.0 {
        OverlayRule::Union
    } else {
        OverlayRule::Difference
    };
    let acc = fill.overlay(&band, rule, FillRule::NonZero);
    let out = shapes_to_subpaths(&acc);
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::path::parse_path_d;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> PathElement {
        let d = format!("M {x0} {y0} L {x1} {y0} L {x1} {y1} L {x0} {y1} Z");
        PathElement {
            id: "r".into(),
            index: 0,
            original_d: d.clone(),
            subpaths: parse_path_d(&d),
            edited: false,
            added: true,
            attributes: None,
            style_override: None,
            original_tag: None,
            deleted: false,
            renamed: false,
            layer: None,
            hidden: false,
        }
    }

    fn bounds(subs: &[Subpath]) -> (f64, f64, f64, f64) {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for sp in subs {
            for n in &sp.nodes {
                min_x = min_x.min(n.point.x);
                min_y = min_y.min(n.point.y);
                max_x = max_x.max(n.point.x);
                max_y = max_y.max(n.point.y);
            }
        }
        (min_x, min_y, max_x, max_y)
    }

    #[test]
    fn union_of_two_overlapping_rects_spans_both() {
        let a = rect(0.0, 0.0, 10.0, 10.0);
        let b = rect(5.0, 5.0, 15.0, 15.0);
        let out = boolean("union", &[&a, &b]).unwrap();
        let (x0, y0, x1, y1) = bounds(&out);
        assert!(
            x0 <= 0.1 && y0 <= 0.1 && x1 >= 14.9 && y1 >= 14.9,
            "{x0},{y0},{x1},{y1}"
        );
    }

    #[test]
    fn intersect_of_two_rects_is_the_overlap() {
        let a = rect(0.0, 0.0, 10.0, 10.0);
        let b = rect(5.0, 5.0, 15.0, 15.0);
        let out = boolean("intersect", &[&a, &b]).unwrap();
        let (x0, y0, x1, y1) = bounds(&out);
        assert!(
            x0 >= 4.9 && y0 >= 4.9 && x1 <= 10.1 && y1 <= 10.1,
            "{x0},{y0},{x1},{y1}"
        );
    }

    #[test]
    fn disjoint_intersect_is_empty() {
        let a = rect(0.0, 0.0, 10.0, 10.0);
        let b = rect(20.0, 20.0, 30.0, 30.0);
        assert!(boolean("intersect", &[&a, &b]).is_none());
    }

    #[test]
    fn unknown_op_is_none() {
        let a = rect(0.0, 0.0, 10.0, 10.0);
        let b = rect(5.0, 5.0, 15.0, 15.0);
        assert!(boolean("nope", &[&a, &b]).is_none());
    }

    #[test]
    fn offset_outward_grows_bounds() {
        let r = rect(10.0, 10.0, 30.0, 30.0);
        let out = offset_path(&r.subpaths, 5.0).unwrap();
        let (x0, y0, x1, y1) = bounds(&out);
        assert!(
            x0 <= 6.0 && y0 <= 6.0 && x1 >= 34.0 && y1 >= 34.0,
            "{x0},{y0},{x1},{y1}"
        );
    }

    #[test]
    fn offset_inward_shrinks_bounds() {
        let r = rect(0.0, 0.0, 20.0, 20.0);
        let out = offset_path(&r.subpaths, -5.0).unwrap();
        let (x0, y0, x1, y1) = bounds(&out);
        assert!(
            x0 >= 4.0 && y0 >= 4.0 && x1 <= 16.0 && y1 <= 16.0,
            "{x0},{y0},{x1},{y1}"
        );
    }
}
