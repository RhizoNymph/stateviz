//! Edge routing.
//!
//! Every edge is routed by its [`EdgeKind`]:
//!
//! - chains through their band's channels (orthogonal with tracks, or
//!   polyline through dummy points);
//! - self-loops as small loops beside their node;
//! - cross-band edges through the gaps between bands, the free passages of
//!   bands in between and, as a last resort, side corridors ([`cross`]);
//! - edges touching pinned nodes by the obstacle router ([`astar`]).
//!
//! Finally every route is checked: one that crosses a node, or enters a
//! foreign group other than straight across it through a passage (which
//! can only happen around pins), is rerouted by the obstacle router.

pub(crate) mod astar;
pub(crate) mod channels;
pub(crate) mod check;
pub(crate) mod cross;
pub(crate) mod paths;

use std::collections::BTreeMap;

use crate::geometry::{Point, Rect};
use crate::options::EdgeRouting;

use super::bands::Placement;
use super::context::{Columns, Ctx};
use super::problem::EdgeKind;
use channels::{Leg, SegKey};
use check::RectIndex;
use cross::{CrossGeometry, CrossPlan, Passage, Request, Stack};

pub(crate) struct Routes {
    /// Canonical polyline of every edge.
    pub points: Vec<Vec<Point>>,
    /// Edges rerouted around obstacles (their label goes on the new route).
    pub rerouted: Vec<bool>,
}

/// Net keys of an edge's explicit ports.
pub(crate) fn port_nets(ctx: &Ctx<'_, '_>, edge: usize) -> [Option<u64>; 2] {
    let e = &ctx.p.edges[edge];
    [e.source.port.map(|p| channels::net(e.source.node, p)), e.target.port.map(|p| channels::net(e.target.node, p))]
}

/// Stack positions, the bands' shared extent and corridor bases for the
/// given band rects.
pub(crate) fn stack_info(ctx: &Ctx<'_, '_>, order: &[usize], outer: &[Option<Rect>]) -> Stack {
    let mut position = vec![None; ctx.bands.len()];
    for (k, &b) in order.iter().enumerate() {
        position[b] = Some(k);
    }
    let (l, r) = order
        .iter()
        .filter_map(|&b| outer[b])
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(l, r), rect| (l.min(rect.left()), r.max(rect.right())));
    let (l, r) = if l.is_finite() { (l, r) } else { (0.0, 0.0) };
    let margin = ctx.p.spacing.edge.max(ctx.p.spacing.group / 2.0);
    Stack {
        position,
        order: order.to_vec(),
        span: (l, r),
        left_base: l - margin,
        right_base: r + margin,
        edge_spacing: ctx.p.spacing.edge,
    }
}

/// Track positions of every band's channel segments, given each band's
/// boundaries.
pub(crate) fn channel_positions(
    ctx: &Ctx<'_, '_>,
    cols: &[Columns],
    stack: &Stack,
    bounds: &[Option<(f32, f32)>],
) -> Vec<BTreeMap<SegKey, f32>> {
    let mut seg_x = vec![BTreeMap::new(); ctx.bands.len()];
    for (b, bound) in bounds.iter().enumerate() {
        let Some((top, bottom)) = *bound else { continue };
        let here = stack.position[b];
        let below = |other: usize| stack.position[other] > here;
        let segs = channels::collect(ctx, b, top, bottom, &below);
        seg_x[b] = match ctx.shared {
            Some(shared) => shared.positions(b, &segs),
            None => {
                let (tracks, counts) = channels::assign_tracks(&segs, cols[b].count() + 1);
                channels::positions(&segs, &tracks, &counts, &cols[b])
            }
        };
    }
    seg_x
}

/// Main-axis position of a cross-band edge's leg in its own band: the
/// attachment point for a direct leg, its channel track otherwise.
pub(crate) fn leg_x(ctx: &Ctx<'_, '_>, seg_x: &[BTreeMap<SegKey, f32>], edge: usize, source: bool) -> Option<f32> {
    let e = &ctx.p.edges[edge];
    let node = e.end(source).node;
    match channels::leg(ctx, edge, source) {
        Leg::Direct => Some(ctx.slots.attach(ctx.p, edge, source, ctx.node_rect(node)).x),
        Leg::Channel => {
            let key = if source { SegKey::Exit { edge } } else { SegKey::Entry { edge } };
            seg_x[ctx.p.nodes[node].band].get(&key).copied()
        }
    }
}

/// Cross-band plans from the positions of their legs, and the passages of
/// every stacked band they were planned through.
pub(crate) fn cross_plans(
    ctx: &Ctx<'_, '_>,
    cols: &[Columns],
    seg_x: &[BTreeMap<SegKey, f32>],
    stack: &Stack,
) -> (Vec<CrossPlan>, Vec<Vec<Passage>>) {
    let p = ctx.p;
    if let Some(shared) = ctx.shared {
        return (super::shared::cross::plans(ctx, shared, seg_x, stack), vec![Vec::new(); ctx.bands.len()]);
    }
    let pins: Vec<(f32, f32)> = p.nodes.iter().filter_map(|n| n.pin.map(|pin| (pin.x, pin.x + n.size.width))).collect();
    let mut passages: Vec<Vec<Passage>> = (0..ctx.bands.len())
        .map(|b| {
            if stack.position[b].is_some() {
                cross::passages::of_band(ctx, b, &cols[b], &seg_x[b], stack.span, &pins)
            } else {
                Vec::new()
            }
        })
        .collect();
    let requests: Vec<Request> = (0..p.edges.len())
        .filter(|&e| p.kinds[e] == EdgeKind::CrossBand)
        .filter_map(|e| {
            Some(Request {
                edge: e,
                source_band: p.nodes[p.edges[e].source.node].band,
                target_band: p.nodes[p.edges[e].target.node].band,
                x_s: leg_x(ctx, seg_x, e, true)?,
                x_t: leg_x(ctx, seg_x, e, false)?,
            })
        })
        .collect();
    let plans = cross::plan_all(&requests, stack, &mut passages);
    (plans, passages)
}

fn reroute(router: &mut astar::Router, ctx: &Ctx<'_, '_>, edge: usize, rects: &[Rect], foreign: &[Rect]) -> Vec<Point> {
    let e = &ctx.p.edges[edge];
    let start = ctx.slots.attach(ctx.p, edge, true, rects[e.source.node]);
    let end = ctx.slots.attach(ctx.p, edge, false, rects[e.target.node]);
    let mut obstacles: Vec<astar::Obstacle> = rects.iter().map(|&rect| astar::Obstacle::Node(rect)).collect();
    obstacles.extend(foreign.iter().map(|&rect| astar::Obstacle::Group(rect)));
    let ends = astar::Ends { start, start_side: e.source.side, end, end_side: e.target.side };
    router.route(ends, &obstacles, ctx.p.spacing.edge.max(4.0))
}

pub(crate) fn route_all(
    ctx: &Ctx<'_, '_>,
    cols: &[Columns],
    placement: &Placement,
    chain_of: &[Option<(usize, usize)>],
) -> Routes {
    let p = ctx.p;
    let stack = stack_info(ctx, &placement.order, &placement.outer);
    let bounds: Vec<Option<(f32, f32)>> = placement.outer.iter().map(|o| o.map(|r| (r.top(), r.bottom()))).collect();
    let seg_x = channel_positions(ctx, cols, &stack, &bounds);
    let (plans, passages) = cross_plans(ctx, cols, &seg_x, &stack);
    let nets = |e: usize| port_nets(ctx, e);
    let geometry = cross::resolve(&plans, &passages, &stack, &placement.gaps, &nets);
    let cross_of: BTreeMap<usize, CrossGeometry> = plans.iter().map(|pl| pl.edge).zip(geometry).collect();

    let rects: Vec<Rect> = (0..p.nodes.len()).map(|v| ctx.node_rect(v)).collect();
    let groups: Vec<(usize, Rect)> = placement
        .order
        .iter()
        .filter(|&&b| p.bands[b].group.is_some())
        .filter_map(|&b| placement.outer[b].map(|r| (b, r)))
        .collect();
    let foreign_of = |edge: usize| -> Vec<Rect> {
        let (sb, tb) = (p.nodes[p.edges[edge].source.node].band, p.nodes[p.edges[edge].target.node].band);
        groups.iter().filter(|(b, _)| *b != sb && *b != tb).map(|(_, r)| *r).collect()
    };

    let mut router = astar::Router::default();
    let mut points = Vec::with_capacity(p.edges.len());
    let mut rerouted = vec![false; p.edges.len()];
    for e in 0..p.edges.len() {
        let route = match (p.kinds[e], chain_of[e], cross_of.get(&e)) {
            (EdgeKind::Chain, Some((band, chain)), _) => match p.routing {
                EdgeRouting::Orthogonal => paths::chain_orthogonal(ctx, band, chain, &seg_x[band]),
                EdgeRouting::Polyline => paths::chain_polyline(ctx, band, chain, &cols[band]),
            },
            (EdgeKind::SelfLoop, _, _) => {
                let band = p.nodes[p.edges[e].source.node].band;
                paths::self_loop(ctx, e, Some(&seg_x[band]))
            }
            (EdgeKind::CrossBand, _, Some(g)) => paths::cross_band(ctx, e, g),
            _ => {
                rerouted[e] = true;
                reroute(&mut router, ctx, e, &rects, &foreign_of(e))
            }
        };
        points.push(route);
    }

    let index = RectIndex::new(rects);
    for e in 0..p.edges.len() {
        if rerouted[e] {
            continue;
        }
        let foreign = foreign_of(e);
        if !check::is_clear(&points[e], &index, &foreign) {
            points[e] = reroute(&mut router, ctx, e, index.rects(), &foreign);

            rerouted[e] = true;
        }
    }
    Routes { points, rerouted }
}
