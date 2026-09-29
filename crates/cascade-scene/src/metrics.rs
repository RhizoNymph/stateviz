//! Readability measurements of a finished [`Scene`], so layout and routing
//! changes can be judged by numbers as well as by eye.
//!
//! All measures count only visible items (opacity > 0) and treat edges as
//! the polylines they are drawn as. Labels are sized with [`MonoMeasure`],
//! the same measure the views use.

use std::fmt;

use cascade_layout::{Point, Rect};

use crate::scene::{HitTarget, Scene, SceneEdge};
use crate::text::{MonoMeasure, TextMeasure};

/// Readability of one scene. Lower is better for every count and length.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SceneMetrics {
    pub nodes: usize,
    pub edges: usize,
    /// Pairs of edges whose segments cross (touching at a shared endpoint
    /// does not count).
    pub edge_crossings: usize,
    /// Sum of all edge polyline lengths, in scene units.
    pub total_edge_length: f32,
    /// Interior polyline points (direction changes) over all edges.
    pub bends: usize,
    /// Edge segments that pass through a node that is not one of the edge's
    /// own endpoints.
    pub edges_through_nodes: usize,
    /// Pairs of labels (edge labels and node labels) whose boxes overlap,
    /// plus edge labels overlapping a node they do not belong to.
    pub label_overlaps: usize,
    /// Edges with a point outside the horizontal extent of every lane:
    /// routed through a side corridor.
    pub corridor_edges: usize,
    /// Bounding box area, in scene units squared.
    pub area: f32,
}

impl fmt::Display for SceneMetrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "nodes {:>4}  edges {:>4}  crossings {:>5}  length {:>9.0}  bends {:>5}  through-nodes {:>4}  \
             label-overlaps {:>4}  corridor {:>4}  area {:>10.0}",
            self.nodes,
            self.edges,
            self.edge_crossings,
            self.total_edge_length,
            self.bends,
            self.edges_through_nodes,
            self.label_overlaps,
            self.corridor_edges,
            self.area
        )
    }
}

fn visible_edges(scene: &Scene) -> Vec<&SceneEdge> {
    scene.edges.iter().filter(|e| e.opacity > 0.0 && e.points.len() >= 2).collect()
}

fn label_rect(text: &str, origin: Point, font_size: f32) -> Rect {
    let m = MonoMeasure::default();
    Rect::from_origin_size(origin, cascade_layout::Size::new(m.width(text, font_size), m.line_height(font_size)))
}

/// Strict segment crossing: proper intersection of interiors.
fn segments_cross(a: Point, b: Point, c: Point, d: Point) -> bool {
    let orient = |p: Point, q: Point, r: Point| (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x);
    let (d1, d2, d3, d4) = (orient(c, d, a), orient(c, d, b), orient(a, b, c), orient(a, b, d));
    let eps = 1e-3;
    ((d1 > eps && d2 < -eps) || (d1 < -eps && d2 > eps)) && ((d3 > eps && d4 < -eps) || (d3 < -eps && d4 > eps))
}

/// Whether segment `a`–`b` passes through the interior of `r` (shrunk by
/// one unit so edges attaching to or grazing a node do not count).
fn segment_through_rect(a: Point, b: Point, r: &Rect) -> bool {
    let inner = Rect::new(r.left() + 1.0, r.top() + 1.0, (r.size.width - 2.0).max(0.0), (r.size.height - 2.0).max(0.0));
    if inner.size.width <= 0.0 || inner.size.height <= 0.0 {
        return false;
    }
    // Orthogonal segments are the common case; sample the segment finely
    // enough for node sizes in use (>= 8 units).
    let len = a.distance(b);
    let steps = (len / 4.0).ceil().max(1.0) as usize;
    (0..=steps).any(|i| {
        let t = i as f32 / steps as f32;
        inner.contains(Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t))
    })
}

fn endpoints_touch(edge: &SceneEdge, rect: &Rect) -> bool {
    let grow = rect.outset(cascade_layout::Insets::uniform(2.0));
    edge.points.first().is_some_and(|p| grow.contains(*p)) || edge.points.last().is_some_and(|p| grow.contains(*p))
}

/// Measure a scene.
pub fn measure(scene: &Scene) -> SceneMetrics {
    let edges = visible_edges(scene);
    let nodes: Vec<&crate::scene::SceneNode> = scene.nodes.iter().filter(|n| n.opacity > 0.0).collect();

    let segments: Vec<Vec<(Point, Point)>> =
        edges.iter().map(|e| e.points.windows(2).map(|w| (w[0], w[1])).collect()).collect();

    let mut edge_crossings = 0;
    for i in 0..segments.len() {
        for j in (i + 1)..segments.len() {
            let crosses =
                segments[i].iter().any(|&(a, b)| segments[j].iter().any(|&(c, d)| segments_cross(a, b, c, d)));
            if crosses {
                edge_crossings += 1;
            }
        }
    }

    let total_edge_length = segments.iter().flat_map(|s| s.iter()).map(|(a, b)| a.distance(*b)).sum::<f32>();

    let bends = edges
        .iter()
        .map(|e| {
            e.points
                .windows(3)
                .filter(|w| {
                    let (a, b, c) = (w[0], w[1], w[2]);
                    let cross = (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x);
                    cross.abs() > 1e-3
                })
                .count()
        })
        .sum();

    let mut edges_through_nodes = 0;
    for (edge, segs) in edges.iter().zip(&segments) {
        for node in &nodes {
            if matches!(node.target, HitTarget::None) || endpoints_touch(edge, &node.rect) {
                continue;
            }
            if segs.iter().any(|&(a, b)| segment_through_rect(a, b, &node.rect)) {
                edges_through_nodes += 1;
            }
        }
    }

    let mut labels: Vec<(Rect, Option<usize>)> = Vec::new();
    for (i, edge) in edges.iter().enumerate() {
        if let Some(l) = &edge.label {
            labels.push((label_rect(&l.text, l.origin, l.font_size), Some(i)));
        }
    }
    let edge_label_count = labels.len();
    for node in &nodes {
        for l in &node.labels {
            labels.push((label_rect(&l.text, l.origin, l.font_size), None));
        }
    }
    let mut label_overlaps = 0;
    for i in 0..labels.len() {
        for j in (i + 1)..labels.len() {
            if labels[i].0.intersects(&labels[j].0) {
                // Two labels of the same node are laid out together.
                if labels[i].1.is_none() && labels[j].1.is_none() {
                    continue;
                }
                label_overlaps += 1;
            }
        }
    }
    for (rect, owner) in labels.iter().take(edge_label_count) {
        let Some(edge_ix) = owner else { continue };
        for node in &nodes {
            if node.rect.intersects(rect) && !endpoints_touch(edges[*edge_ix], &node.rect) {
                label_overlaps += 1;
            }
        }
    }

    let corridor_edges = if scene.lanes.is_empty() {
        0
    } else {
        let left = scene.lanes.iter().map(|l| l.rect.left()).fold(f32::INFINITY, f32::min);
        let right = scene.lanes.iter().map(|l| l.rect.right()).fold(f32::NEG_INFINITY, f32::max);
        edges.iter().filter(|e| e.points.iter().any(|p| p.x < left - 0.5 || p.x > right + 0.5)).count()
    };

    SceneMetrics {
        nodes: nodes.len(),
        edges: edges.len(),
        edge_crossings,
        total_edge_length,
        bends,
        edges_through_nodes,
        label_overlaps,
        corridor_edges,
        area: scene.bounds.size.width * scene.bounds.size.height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_crossing() {
        let p = Point::new;
        assert!(segments_cross(p(0.0, 5.0), p(10.0, 5.0), p(5.0, 0.0), p(5.0, 10.0)));
        assert!(!segments_cross(p(0.0, 0.0), p(10.0, 0.0), p(10.0, 0.0), p(10.0, 10.0)));
        assert!(!segments_cross(p(0.0, 0.0), p(10.0, 0.0), p(0.0, 5.0), p(10.0, 5.0)));
    }

    #[test]
    fn through_rect() {
        let r = Rect::new(10.0, 10.0, 20.0, 20.0);
        assert!(segment_through_rect(Point::new(0.0, 20.0), Point::new(40.0, 20.0), &r));
        assert!(!segment_through_rect(Point::new(0.0, 5.0), Point::new(40.0, 5.0), &r));
    }
}
