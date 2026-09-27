//! Label boxes.
//!
//! - A chain edge's label sits in the room its label dummy reserved: a box
//!   as wide as the label in the dummy's column, just above the edge line.
//! - A self-loop's label sits above its node, stacked over any other loop
//!   labels, in room the node's top margin reserved.
//! - Anything else (a cross-band edge, one touching a pinned node, one
//!   rerouted around an obstacle, or a chain too short for a label column)
//!   gets its label beside the middle of one of its segments: the longest
//!   one where the box clears every node (above or below a horizontal
//!   segment, right or left of a vertical one).

use crate::geometry::{Point, Rect, Size};

use super::context::{Columns, Ctx};
use super::layered::ItemKind;
use super::problem::EdgeKind;
use super::routing::check::RectIndex;

/// Gap between a label box and its edge line.
pub(crate) fn label_gap(edge_spacing: f32) -> f32 {
    (edge_spacing / 2.0).max(2.0)
}

fn fallback(points: &[Point], size: Size, gap: f32, nodes: &RectIndex) -> Rect {
    let mut segments: Vec<(Point, Point)> = points.windows(2).map(|w| (w[0], w[1])).collect();
    let len = |s: &(Point, Point)| (s.0.x - s.1.x).abs() + (s.0.y - s.1.y).abs();
    segments.sort_by(|a, b| len(b).total_cmp(&len(a)));
    let mut candidates = Vec::with_capacity(segments.len() * 2);
    for (a, b) in segments {
        let (mx, my) = ((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
        if (a.y - b.y).abs() < 1e-3 {
            candidates.push(Rect::new(mx - size.width / 2.0, a.y - gap - size.height, size.width, size.height));
            candidates.push(Rect::new(mx - size.width / 2.0, a.y + gap, size.width, size.height));
        } else {
            candidates.push(Rect::new(a.x + gap, my - size.height / 2.0, size.width, size.height));
            candidates.push(Rect::new(a.x - gap - size.width, my - size.height / 2.0, size.width, size.height));
        }
    }
    candidates.iter().find(|c| !nodes.overlaps(c)).or(candidates.first()).copied().unwrap_or_else(|| {
        let p = points.first().copied().unwrap_or_default();
        Rect::new(p.x, p.y - gap - size.height, size.width, size.height)
    })
}

/// Canonical label box of every edge (`None` without a label).
pub(crate) fn boxes(
    ctx: &Ctx<'_, '_>,
    routes: &[Vec<crate::geometry::Point>],
    chain_of: &[Option<(usize, usize)>],
    cols: &[Columns],
    rerouted: &[bool],
) -> Vec<Option<Rect>> {
    let p = ctx.p;
    let gap = label_gap(p.spacing.edge);
    let nodes = RectIndex::new((0..p.nodes.len()).map(|v| ctx.node_rect(v)).collect());
    let mut loop_cursor: Vec<Option<f32>> = vec![None; p.nodes.len()];
    let mut out = Vec::with_capacity(p.edges.len());
    for (e, edge) in p.edges.iter().enumerate() {
        let Some(size) = edge.label else {
            out.push(None);
            continue;
        };
        let reserved = match (p.kinds[e], chain_of[e]) {
            (EdgeKind::Chain, Some((band, chain))) if !rerouted[e] => {
                let bg = &ctx.bands[band];
                bg.chains[chain].items.iter().find_map(|&i| {
                    let it = &bg.items[i];
                    matches!(it.kind, ItemKind::Label { .. }).then(|| {
                        Rect::new(cols[band].centre(it.layer) - size.width / 2.0, it.top, size.width, size.height)
                    })
                })
            }
            (EdgeKind::SelfLoop, _) if !rerouted[e] && ctx.item(edge.source.node).is_some() => {
                let v = edge.source.node;
                let rect = ctx.node_rect(v);
                let levels = ctx.slots.north_levels(v);
                let base = if levels == 0 { 0.0 } else { (levels as f32 + 0.5) * p.spacing.edge };
                let y = loop_cursor[v].unwrap_or(rect.top() - base) - gap - size.height;
                loop_cursor[v] = Some(y);
                Some(Rect::new(rect.center().x - size.width / 2.0, y, size.width, size.height))
            }
            _ => None,
        };
        out.push(Some(reserved.unwrap_or_else(|| fallback(&routes[e], size, gap, &nodes))));
    }
    out
}
