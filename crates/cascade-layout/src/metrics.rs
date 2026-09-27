//! Measurements of a finished layout, for tests, benchmarks and
//! diagnostics.

use crate::geometry::Point;
use crate::graph::{LayoutGraph, NodeId};
use crate::result::LayoutResult;

const EPS: f32 = 1e-3;

fn orient(a: Point, b: Point, c: Point) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

/// Whether segments `a`–`b` and `c`–`d` cross at a single point interior to
/// both (touching, sharing an end, or running along each other is not a
/// crossing).
pub fn segments_cross(a: Point, b: Point, c: Point, d: Point) -> bool {
    let sign = |v: f32| {
        if v > EPS {
            1
        } else if v < -EPS {
            -1
        } else {
            0
        }
    };
    let (o1, o2) = (sign(orient(a, b, c)), sign(orient(a, b, d)));
    let (o3, o4) = (sign(orient(c, d, a)), sign(orient(c, d, b)));
    o1 * o2 < 0 && o3 * o4 < 0
}

/// Number of crossings between the routes of different edges.
pub fn count_crossings(graph: &LayoutGraph, result: &LayoutResult) -> usize {
    let segments: Vec<(usize, Point, Point)> = graph
        .edges()
        .flat_map(|(id, _)| {
            result.edge(id).points.windows(2).map(move |w| (id.index(), w[0], w[1])).collect::<Vec<_>>()
        })
        .collect();
    let mut count = 0;
    for (i, &(ea, a, b)) in segments.iter().enumerate() {
        let (ax0, ax1, ay0, ay1) = (a.x.min(b.x), a.x.max(b.x), a.y.min(b.y), a.y.max(b.y));
        for &(eb, c, d) in &segments[i + 1..] {
            if ea == eb {
                continue;
            }
            if c.x.max(d.x) < ax0 || c.x.min(d.x) > ax1 || c.y.max(d.y) < ay0 || c.y.min(d.y) > ay1 {
                continue;
            }
            if segments_cross(a, b, c, d) {
                count += 1;
            }
        }
    }
    count
}

/// Crossings between edges joining neighbouring layers of the same group,
/// judged from each node's `layer` and `order` alone. Useful for comparing
/// orderings independently of routing.
pub fn count_order_crossings(graph: &LayoutGraph, result: &LayoutResult) -> usize {
    let spans: Vec<(Option<usize>, u32, i64, i64)> = graph
        .edges()
        .filter_map(|(_, e)| {
            let (s, t) = (result.node(e.source.node), result.node(e.target.node));
            let group = graph.node(e.source.node).group;
            (group == graph.node(e.target.node).group && t.layer == s.layer.checked_add(1)?)
                .then(|| (group.map(|g| g.index()), s.layer, i64::from(s.order), i64::from(t.order)))
        })
        .collect();
    let mut count = 0;
    for (i, a) in spans.iter().enumerate() {
        for b in &spans[i + 1..] {
            if a.0 == b.0 && a.1 == b.1 && (a.2 - b.2) * (a.3 - b.3) < 0 {
                count += 1;
            }
        }
    }
    count
}

/// Pairs of nodes whose rects overlap (touching does not count).
pub fn overlapping_nodes(graph: &LayoutGraph, result: &LayoutResult) -> Vec<(NodeId, NodeId)> {
    let ids: Vec<NodeId> = graph.nodes().map(|(id, _)| id).collect();
    let mut out = Vec::new();
    for (i, &a) in ids.iter().enumerate() {
        for &b in &ids[i + 1..] {
            if result.node(a).rect.intersects(&result.node(b).rect) {
                out.push((a, b));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossing_is_proper_intersection_only() {
        let p = Point::new;
        assert!(segments_cross(p(0.0, 5.0), p(10.0, 5.0), p(5.0, 0.0), p(5.0, 10.0)));
        assert!(!segments_cross(p(0.0, 5.0), p(10.0, 5.0), p(10.0, 0.0), p(10.0, 10.0)));
        assert!(!segments_cross(p(0.0, 5.0), p(10.0, 5.0), p(2.0, 5.0), p(8.0, 5.0)));
        assert!(!segments_cross(p(0.0, 0.0), p(10.0, 0.0), p(0.0, 1.0), p(10.0, 1.0)));
    }
}
