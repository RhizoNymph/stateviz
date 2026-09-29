//! Label boxes, placed so that no label overlaps a node, a group's header
//! strip or another label.
//!
//! - A chain edge's label goes in the room its label dummy reserved: a box
//!   as wide as the label in the dummy's column, just above the edge line.
//! - A self-loop's label goes above its node, stacked over any other loop
//!   labels, in room the node's top margin reserved.
//! - Anything else (a cross-band edge, one touching a pinned node, one
//!   rerouted around an obstacle, or a chain too short for a label column),
//!   and a reserved box that turns out to collide (around pins), gets a box
//!   beside one of its segments: longer segments first, starting at the
//!   segment's middle and sliding toward its ends, on either side. The
//!   first box clear of nodes, headers, placed labels and other edges'
//!   routes wins; failing that, the first clear of nodes, headers and
//!   labels; failing that, the one overlapping least, which is reported as
//!   unplaced.
//!
//! Reserved labels are placed first, then the others in edge order.

use std::collections::HashMap;

use crate::geometry::{Point, Rect, Size};

use super::context::{Columns, Ctx};
use super::layered::ItemKind;
use super::problem::EdgeKind;
use super::routing::check::{RectIndex, SHRINK, segment_hits};

/// Gap between a label box and its edge line.
pub(crate) fn label_gap(edge_spacing: f32) -> f32 {
    (edge_spacing / 2.0).max(2.0)
}

/// Positions tried along one segment.
const SLIDES: usize = 24;
/// Room a label keeps from nodes, headers and other labels, so that
/// rounding never makes them touch.
const CLEARANCE: f32 = 0.5;

fn grown(r: &Rect, by: f32) -> Rect {
    r.outset(crate::geometry::Insets::uniform(by))
}

fn overlap_area(a: &Rect, b: &Rect) -> f32 {
    let w = a.right().min(b.right()) - a.left().max(b.left());
    let h = a.bottom().min(b.bottom()) - a.top().max(b.top());
    if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
}

/// Candidate boxes beside the route's segments, best first.
fn candidates(points: &[Point], size: Size, gap: f32) -> Vec<Rect> {
    let mut segments: Vec<(usize, Point, Point)> =
        points.windows(2).enumerate().map(|(i, w)| (i, w[0], w[1])).collect();
    let len = |a: Point, b: Point| (a.x - b.x).abs() + (a.y - b.y).abs();
    segments.sort_by(|x, y| len(y.1, y.2).total_cmp(&len(x.1, x.2)).then(x.0.cmp(&y.0)));
    let mut out = Vec::new();
    for (_, a, b) in segments {
        let horizontal = (a.y - b.y).abs() < 1e-3;
        let (lo, hi, along) = if horizontal {
            (a.x.min(b.x), a.x.max(b.x), size.width)
        } else {
            (a.y.min(b.y), a.y.max(b.y), size.height)
        };
        let mid = (lo + hi) / 2.0;
        // Centres from the middle outward, keeping the box beside the
        // segment when it is long enough.
        let reach = ((hi - lo - along) / 2.0).max(0.0);
        let step = (along / 4.0).max(reach / SLIDES as f32).max(1.0);
        let mut centres = vec![mid];
        let mut d = step;
        while d <= reach + 1e-3 {
            centres.push(mid - d);
            centres.push(mid + d);
            d += step;
        }
        for c in centres {
            if horizontal {
                let left = c - size.width / 2.0;
                out.push(Rect::new(left, a.y - gap - size.height, size.width, size.height));
                out.push(Rect::new(left, a.y + gap, size.width, size.height));
            } else {
                let top = c - size.height / 2.0;
                out.push(Rect::new(a.x + gap, top, size.width, size.height));
                out.push(Rect::new(a.x - gap - size.width, top, size.width, size.height));
            }
        }
    }
    if out.is_empty() {
        let p = points.first().copied().unwrap_or_default();
        out.push(Rect::new(p.x, p.y - gap - size.height, size.width, size.height));
    }
    out
}

/// A route segment and the edge it belongs to.
type Tagged = (usize, Point, Point);

/// Uniform-grid index of route segments, for "does this box cover another
/// edge's route" queries.
struct SegmentIndex {
    cell: f32,
    buckets: HashMap<(i64, i64), Vec<Tagged>>,
}

impl SegmentIndex {
    const CELL: f32 = 96.0;

    fn cell_of(&self, v: f32) -> i64 {
        (v / self.cell).floor().clamp(-1e9, 1e9) as i64
    }

    fn new(routes: &[Vec<Point>]) -> Self {
        let mut index = Self { cell: Self::CELL, buckets: HashMap::new() };
        for (e, pts) in routes.iter().enumerate() {
            for w in pts.windows(2) {
                let (x0, x1) = (index.cell_of(w[0].x.min(w[1].x)), index.cell_of(w[0].x.max(w[1].x)));
                let (y0, y1) = (index.cell_of(w[0].y.min(w[1].y)), index.cell_of(w[0].y.max(w[1].y)));
                // Oversized segments (degenerate geometry) go in one bucket
                // per end rather than flooding the index.
                if (x1 - x0 + 1).saturating_mul(y1 - y0 + 1) > 4096 {
                    for p in [w[0], w[1]] {
                        index
                            .buckets
                            .entry((index.cell_of(p.x), index.cell_of(p.y)))
                            .or_default()
                            .push((e, w[0], w[1]));
                    }
                    continue;
                }
                for cx in x0..=x1 {
                    for cy in y0..=y1 {
                        index.buckets.entry((cx, cy)).or_default().push((e, w[0], w[1]));
                    }
                }
            }
        }
        index
    }

    /// Whether a segment of an edge other than `own` runs through `r`.
    fn covers(&self, r: &Rect, own: usize) -> bool {
        let (x0, x1) = (self.cell_of(r.left()), self.cell_of(r.right()));
        let (y0, y1) = (self.cell_of(r.top()), self.cell_of(r.bottom()));
        (x0..=x1).any(|cx| {
            (y0..=y1).any(|cy| {
                self.buckets
                    .get(&(cx, cy))
                    .is_some_and(|b| b.iter().any(|&(e, a, z)| e != own && segment_hits(a, z, r, 0.0)))
            })
        })
    }
}

/// Everything a label must stay clear of, and the routes it would rather
/// not cover.
struct Field {
    nodes: RectIndex,
    headers: Vec<Rect>,
    placed: Vec<Rect>,
    routes: SegmentIndex,
}

impl Field {
    /// Total area of hard overlaps (nodes, headers, placed labels).
    fn clash(&self, r: &Rect) -> f32 {
        let s = grown(r, CLEARANCE);
        let mut area: f32 = self.headers.iter().chain(&self.placed).map(|o| overlap_area(&s, o)).sum();
        // The index shrinks nodes by its own tolerance before testing.
        if self.nodes.overlaps(&grown(r, CLEARANCE + SHRINK)) {
            area += self.nodes.rects().iter().map(|o| overlap_area(&s, o)).sum::<f32>().max(1.0);
        }
        area
    }

    /// Whether another edge's route runs through the box.
    fn covers_route(&self, r: &Rect, own: usize) -> bool {
        self.routes.covers(r, own)
    }

    /// The best of `cands` and whether it is clear of every hard obstacle.
    fn pick(&self, cands: &[Rect], own: usize) -> (Rect, bool) {
        let mut clear_but_covering: Option<Rect> = None;
        let mut least: Option<(f32, Rect)> = None;
        for c in cands {
            let clash = self.clash(c);
            if clash <= 0.0 {
                if !self.covers_route(c, own) {
                    return (*c, true);
                }
                clear_but_covering.get_or_insert(*c);
            } else if least.is_none_or(|(a, _)| clash < a) {
                least = Some((clash, *c));
            }
        }
        match (clear_but_covering, least) {
            (Some(c), _) => (c, true),
            (None, Some((_, c))) => (c, false),
            (None, None) => (cands.first().copied().unwrap_or_default(), false),
        }
    }
}

/// Canonical header strip of every stacked group: its padding and header
/// band on the real top.
fn header_strips(ctx: &Ctx<'_, '_>, outer: &[Option<Rect>]) -> Vec<Rect> {
    let p = ctx.p;
    let frame = p.frame;
    (0..p.bands.len())
        .filter_map(|b| {
            let g = p.bands[b].group?;
            let group = p.graph.group(crate::graph::GroupId::new(g));
            let real = frame.rect((*outer.get(b)?)?);
            let strip = Rect::new(
                real.left(),
                real.top(),
                real.size.width,
                (group.padding.top + group.header).min(real.size.height),
            );
            Some(frame.rect(strip))
        })
        .filter(|r| r.size.width > 0.0 && r.size.height > 0.0)
        .collect()
}

/// Canonical label box of every edge (`None` without a label), and how
/// many labels could not be placed cleanly.
pub(crate) fn boxes(
    ctx: &Ctx<'_, '_>,
    routes: &[Vec<Point>],
    chain_of: &[Option<(usize, usize)>],
    cols: &[Columns],
    outer: &[Option<Rect>],
    rerouted: &[bool],
) -> (Vec<Option<Rect>>, usize) {
    let p = ctx.p;
    let gap = label_gap(p.spacing.edge);
    let mut field = Field {
        nodes: RectIndex::new((0..p.nodes.len()).map(|v| ctx.node_rect(v)).collect()),
        headers: header_strips(ctx, outer),
        placed: Vec::new(),
        routes: SegmentIndex::new(routes),
    };

    // Room reserved in the layout.
    let mut loop_cursor: Vec<Option<f32>> = vec![None; p.nodes.len()];
    let reserved: Vec<Option<Rect>> = p
        .edges
        .iter()
        .enumerate()
        .map(|(e, edge)| {
            let size = edge.label?;
            match (p.kinds[e], chain_of[e]) {
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
            }
        })
        .collect();

    let mut out: Vec<Option<Rect>> = vec![None; p.edges.len()];
    let mut pending: Vec<usize> = Vec::new();
    for (e, r) in reserved.iter().enumerate() {
        match r {
            Some(r) if field.clash(r) <= 0.0 => {
                field.placed.push(*r);
                out[e] = Some(*r);
            }
            _ if p.edges[e].label.is_some() => pending.push(e),
            _ => {}
        }
    }
    let mut unplaced = 0;
    for e in pending {
        let Some(size) = p.edges[e].label else { continue };
        let mut cands: Vec<Rect> = reserved[e].into_iter().collect();
        cands.extend(candidates(&routes[e], size, gap));
        let (r, clean) = field.pick(&cands, e);
        unplaced += usize::from(!clean);
        field.placed.push(r);
        out[e] = Some(r);
    }
    (out, unplaced)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_start_mid_segment_and_slide() {
        let pts = [Point::new(0.0, 0.0), Point::new(200.0, 0.0), Point::new(200.0, 40.0)];
        let c = candidates(&pts, Size::new(40.0, 10.0), 2.0);
        assert_eq!(c[0], Rect::new(80.0, -12.0, 40.0, 10.0));
        assert_eq!(c[1], Rect::new(80.0, 2.0, 40.0, 10.0));
        assert!(c.iter().any(|r| r.left() < 20.0) && c.iter().any(|r| r.right() > 180.0));
        // The shorter vertical segment comes later, beside it.
        assert!(c.iter().any(|r| r.left() == 202.0));
    }
}
