//! Measurements of a finished layout, for tests, benchmarks and
//! diagnostics.
//!
//! [`measure`] gathers the readability measures ([`RouteMetrics`]); the
//! single measures are public too.

use std::fmt;

use crate::geometry::{Point, Rect};
use crate::graph::{LayoutGraph, NodeId};
use crate::options::FlowDirection;
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

/// Total length of every edge route.
pub fn total_length(graph: &LayoutGraph, result: &LayoutResult) -> f32 {
    graph.edges().flat_map(|(id, _)| result.edge(id).points.windows(2).map(|w| w[0].distance(w[1]))).sum()
}

/// Direction changes over every edge route.
pub fn bends(graph: &LayoutGraph, result: &LayoutResult) -> usize {
    graph
        .edges()
        .map(|(id, _)| {
            result
                .edge(id)
                .points
                .windows(3)
                .filter(|w| orient(w[0], w[1], w[2]).abs() > EPS * (1.0 + w[0].distance(w[1]) + w[1].distance(w[2])))
                .count()
        })
        .sum()
}

/// Edges with a point beside the stack of groups (left or right of every
/// group with left-to-right flow, above or below with top-to-bottom flow):
/// routed through a side corridor. Zero without groups.
pub fn corridor_edges(graph: &LayoutGraph, result: &LayoutResult, direction: FlowDirection) -> usize {
    let rects: Vec<Rect> = graph.groups().map(|(id, _)| result.group(id)).collect();
    let Some(first) = rects.first() else { return 0 };
    let across = |r: &Rect| match direction {
        FlowDirection::LeftToRight => (r.left(), r.right()),
        FlowDirection::TopToBottom => (r.top(), r.bottom()),
    };
    let (lo, hi) = rects.iter().map(across).fold(across(first), |(lo, hi), (a, b)| (lo.min(a), hi.max(b)));
    let coord = |p: &Point| match direction {
        FlowDirection::LeftToRight => p.x,
        FlowDirection::TopToBottom => p.y,
    };
    graph
        .edges()
        .filter(|(id, _)| result.edge(*id).points.iter().any(|p| coord(p) < lo - 0.5 || coord(p) > hi + 0.5))
        .count()
}

/// The header strip of every group: its padding and header band on the
/// real top, where the lane's title goes.
pub fn group_headers(graph: &LayoutGraph, result: &LayoutResult) -> Vec<Rect> {
    graph
        .groups()
        .map(|(id, g)| {
            let r = result.group(id);
            Rect::new(r.left(), r.top(), r.size.width, (g.padding.top + g.header).min(r.size.height))
        })
        .collect()
}

/// Label boxes overlapping another label box, a node, or a group's header
/// strip (each overlapping pair counts once).
pub fn label_overlaps(graph: &LayoutGraph, result: &LayoutResult) -> usize {
    let shrink = |r: &Rect| Rect::new(r.left() + 0.5, r.top() + 0.5, r.size.width - 1.0, r.size.height - 1.0);
    let labels: Vec<Rect> = graph.edges().filter_map(|(id, _)| result.edge(id).label).map(|r| shrink(&r)).collect();
    let mut obstacles: Vec<Rect> = graph.nodes().map(|(id, _)| result.node(id).rect).collect();
    obstacles.extend(group_headers(graph, result));
    let mut count = 0;
    for (i, a) in labels.iter().enumerate() {
        count += labels[i + 1..].iter().filter(|b| a.intersects(b)).count();
        count += obstacles.iter().filter(|o| a.intersects(o)).count();
    }
    count
}

/// Readability measures of one layout. Lower is better for every field.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RouteMetrics {
    pub edges: usize,
    /// Proper crossings between different edges' routes.
    pub crossings: usize,
    /// Edges routed through a side corridor.
    pub corridor_edges: usize,
    pub total_length: f32,
    pub bends: usize,
    /// See [`label_overlaps`].
    pub label_overlaps: usize,
    /// Labels the engine reported it could not place cleanly.
    pub unplaced_labels: usize,
}

impl fmt::Display for RouteMetrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "edges {:>4}  crossings {:>5}  corridor {:>3}  length {:>8.0}  bends {:>5}  label-overlaps {:>3}  unplaced {:>3}",
            self.edges,
            self.crossings,
            self.corridor_edges,
            self.total_length,
            self.bends,
            self.label_overlaps,
            self.unplaced_labels
        )
    }
}

/// Every readability measure of a layout.
pub fn measure(graph: &LayoutGraph, result: &LayoutResult, direction: FlowDirection) -> RouteMetrics {
    RouteMetrics {
        edges: graph.edge_count(),
        crossings: count_crossings(graph, result),
        corridor_edges: corridor_edges(graph, result, direction),
        total_length: total_length(graph, result),
        bends: bends(graph, result),
        label_overlaps: label_overlaps(graph, result),
        unplaced_labels: result.unplaced_labels,
    }
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
