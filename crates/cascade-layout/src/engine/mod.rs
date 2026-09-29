//! The layout entry point: a layered (Sugiyama-style) pipeline.
//!
//! ```text
//! Problem::build        validate, transpose to the canonical left-to-right frame,
//!                       resolve sides, split nodes into bands (groups)
//! layering (per band)   cycles → constrained network-simplex layers
//! layered::build        columns, dummies, chains
//! Slots::plan           port slots, stub levels, node margins
//! ordering              crossing minimisation (fresh bands)
//! stability             previous positions + free-space insertion (stable bands)
//! coordinates           cross-axis L1 placement, column x (fresh bands)
//! bands::stack          stack bands, push items off pins, group rects
//! routing               channel tracks, cross-band gaps/corridors, loops,
//!                       obstacle router for pins, repair
//! labels, output        label boxes, transpose back, layer/order, bounds
//! ```
//!
//! Everything is deterministic: indices are processed in order, ties break
//! by index, and hash maps are only used for lookups.

mod align;
mod bands;
mod context;
mod coordinates;
mod cycles;
mod frame;
mod labels;
mod layered;
mod layering;
mod network_simplex;
mod ordering;
mod packing;
mod problem;
mod routing;
mod shared;
mod slots;
mod stability;
mod tracks;

use crate::geometry::Rect;
use crate::graph::{EdgeId, LayoutGraph};
use crate::options::{LayoutHints, LayoutOptions};
use crate::result::{EdgeRoute, LayoutResult, NodePlacement};

use context::{Columns, Ctx};
use layered::{BandGraph, ItemKind};
use layering::LayerEdge;
use packing::Occupancy;
use problem::{EdgeKind, Mode, Problem};
use slots::Slots;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    /// Two constraints cannot both hold: an edge that lies on no cycle runs
    /// between two nodes whose `First`/`Exact` layers force it to point
    /// backwards (or sideways).
    #[error("cannot satisfy layout constraints for `{key}`: {reason}")]
    Unsatisfiable { key: String, reason: String },
    /// A node's size is negative, infinite or NaN.
    #[error("node `{key}` has a negative or non-finite size")]
    InvalidNodeSize { key: String },
    /// An edge's label size is negative, infinite or NaN.
    #[error("edge {} has a negative or non-finite label size", .edge.index())]
    InvalidEdgeLabel { edge: EdgeId },
    /// A group's padding or header is negative, infinite or NaN.
    #[error("group `{key}` has a negative or non-finite padding or header")]
    InvalidGroup { key: String },
    /// A spacing option is negative, infinite or NaN.
    #[error("layout option `{name}` must be finite and non-negative")]
    InvalidOption { name: &'static str },
    /// A pin is not a finite point.
    #[error("the pin for node `{key}` is not a finite point")]
    InvalidPin { key: String },
}

/// Lay out `graph`. Deterministic: the same inputs always give the same
/// output.
///
/// With `hints.previous`, nodes keep their previous layer, relative order
/// and position wherever the graph allows, so adding or removing one node
/// or edge leaves unrelated nodes exactly where they were. Nodes in
/// `hints.pins` are placed exactly at their pin; other nodes are pushed off
/// them and edges route to their ports.
pub fn layout(graph: &LayoutGraph, options: &LayoutOptions, hints: &LayoutHints) -> Result<LayoutResult, LayoutError> {
    let p = Problem::build(graph, options, hints)?;
    let (layer_of, on_cycle) = if p.shared { shared::layering::assign(&p)? } else { layer_bands(&p)? };
    let es = p.spacing.edge;
    let label_gap = labels::label_gap(es);

    let mut item_of = vec![None; p.nodes.len()];
    let mut bands: Vec<BandGraph> =
        (0..p.bands.len()).map(|b| layered::build(&p, b, &layer_of, &mut item_of, label_gap)).collect();
    let mut chain_of: Vec<Option<(usize, usize)>> = vec![None; p.edges.len()];
    for (b, bg) in bands.iter().enumerate() {
        for (c, chain) in bg.chains.iter().enumerate() {
            chain_of[chain.edge] = Some((b, c));
        }
    }

    let heads_right = |e: usize, source: bool| -> Option<bool> {
        match p.kinds[e] {
            EdgeKind::Chain => {
                let (b, c) = chain_of[e]?;
                let chain = &bands[b].chains[c];
                let (item, channel) = if source {
                    (*chain.items.first()?, *chain.channels.first()?)
                } else {
                    (*chain.items.last()?, *chain.channels.last()?)
                };
                Some(channel > bands[b].items[item].layer)
            }
            EdgeKind::CrossBand => Some(source),
            EdgeKind::SelfLoop | EdgeKind::Pinned => None,
        }
    };
    let mut slots = Slots::plan(&p, &heads_right, label_gap);
    for bg in &mut bands {
        for it in &mut bg.items {
            if let ItemKind::Node(v) = it.kind {
                let (top, bottom) = slots.margins(v, es);
                it.margin_top = top;
                it.margin_bottom = bottom;
                it.reserve = it.width.max(slots.loop_label_width(v));
            }
        }
    }

    // Which bands keep their previous layout, and how.
    let anchors: Vec<stability::Anchors> =
        bands
            .iter()
            .map(|bg| {
                if p.mode == Mode::Stable { stability::anchors(bg, &p, &layer_of) } else { vec![None; bg.items.len()] }
            })
            .collect();
    let seed = |v: usize| if p.mode == Mode::Seeded { p.nodes[v].prev.map(|pr| pr.order) } else { None };
    let mut modes = vec![BandMode::Fresh; bands.len()];
    let mut trials: Vec<Option<BandGraph>> = vec![None; bands.len()];
    for (b, bg) in bands.iter_mut().enumerate() {
        if anchors[b].iter().any(Option::is_some) {
            // Try a fresh layout first: if it reproduces the previous node
            // positions the band is unchanged and is kept exactly.
            let mut trial = bg.clone();
            ordering::minimize(&mut trial, &p, &slots, &seed);
            trials[b] = Some(trial);
            modes[b] = BandMode::Inserted;
        } else {
            ordering::minimize(bg, &p, &slots, &seed);
        }
    }
    if trials.iter().any(Option::is_some) {
        let fresh: Vec<BandGraph> =
            bands.iter().zip(&trials).map(|(bg, trial)| trial.as_ref().unwrap_or(bg).clone()).collect();
        let keys = implicit_keys(&p, &fresh, &chain_of, &vec![false; bands.len()]);
        let mut trial_slots = slots.clone();
        trial_slots.order_implicit(&p, &|e, source| keys[e][usize::from(!source)]);
        for (b, trial) in trials.iter_mut().enumerate() {
            if let Some(mut trial) = trial.take() {
                coordinates::assign_y(&mut trial, &p, &trial_slots);
                if stability::reproduce_y(&mut trial, &anchors[b]) {
                    bands[b] = trial;
                    modes[b] = BandMode::Reproduced;
                }
            }
        }
    }

    let mut occupancy: Vec<Vec<Occupancy>> = vec![Vec::new(); bands.len()];
    for (b, bg) in bands.iter_mut().enumerate() {
        if modes[b] == BandMode::Inserted {
            occupancy[b] = stability::place_nodes(bg, &p, &anchors[b]);
        }
    }

    let inserted: Vec<bool> = modes.iter().map(|m| *m == BandMode::Inserted).collect();
    let keys = implicit_keys(&p, &bands, &chain_of, &inserted);
    slots.order_implicit(&p, &|e, source| keys[e][usize::from(!source)]);

    for (b, bg) in bands.iter_mut().enumerate() {
        match modes[b] {
            BandMode::Fresh => coordinates::assign_y(bg, &p, &slots),
            BandMode::Reproduced => {}
            BandMode::Inserted => stability::place_dummies(bg, &p, &slots, &mut occupancy[b]),
        }
    }

    // Main axis: provisional tracks size the channels.
    let (mut cols, shared) = if p.shared {
        let (shared, cols) = shared::columns::place(&p, &mut bands, &item_of, &slots, &anchors);
        (cols, Some(shared))
    } else {
        (main_axis(&p, &mut bands, &item_of, &slots, &anchors, &modes), None)
    };
    let shared = shared.as_ref();

    if p.align {
        let movable: Vec<bool> = modes.iter().map(|m| *m == BandMode::Fresh).collect();
        let shifts = {
            let ctx = Ctx { p: &p, bands: &bands, item_of: &item_of, slots: &slots, shared };
            align::shifts(&ctx, &cols, &bands::stacked_bands(&p), &movable)
        };
        for (b, shift) in shifts.iter().enumerate() {
            if shift.iter().all(|d| d.abs() < 1e-4) {
                continue;
            }
            let bg = &mut bands[b];
            for (l, &d) in shift.iter().enumerate() {
                for &i in &bg.layers[l] {
                    bg.items[i].x += d;
                }
            }
            cols[b] = Columns::of(bg, cols[b].entry, cols[b].exit);
        }
    }

    // Gap sizes from provisional cross-band tracks, then stacking.
    let gap_tracks = {
        let ctx = Ctx { p: &p, bands: &bands, item_of: &item_of, slots: &slots, shared };
        let order = bands::stacked_bands(&p);
        let outer: Vec<Option<Rect>> = (0..bands.len())
            .map(|b| bands::content_box(&bands[b], &cols[b]).map(|c| c.outset(p.bands[b].insets)))
            .collect();
        let stack = routing::stack_info(&ctx, &order, &outer);
        let bounds: Vec<Option<(f32, f32)>> = outer.iter().map(|o| o.map(|r| (r.top(), r.bottom()))).collect();
        let seg_x = routing::channel_positions(&ctx, &cols, &stack, &bounds);
        let (plans, passages) = routing::cross_plans(&ctx, &cols, &seg_x, &stack);
        routing::cross::gap_track_counts(&plans, &passages, &stack, order.len().saturating_sub(1), &|e| {
            routing::port_nets(&ctx, e)
        })
    };
    let kept: Vec<bool> = modes.iter().map(|m| *m != BandMode::Fresh).collect();
    let placement = bands::stack(&p, &mut bands, &cols, &kept, &gap_tracks);

    let ctx = Ctx { p: &p, bands: &bands, item_of: &item_of, slots: &slots, shared };
    let routes = routing::route_all(&ctx, &cols, &placement, &chain_of);
    let (label_boxes, unplaced) =
        labels::boxes(&ctx, &routes.points, &chain_of, &cols, &placement.outer, &routes.rerouted);
    Ok(assemble(&ctx, &layer_of, &on_cycle, &placement, routes, label_boxes).with_unplaced_labels(unplaced))
}

/// Main-axis positions of every band laid out on its own, by its mode:
/// provisional channel tracks size the channels.
fn main_axis(
    p: &Problem<'_>,
    bands: &mut [BandGraph],
    item_of: &[Option<usize>],
    slots: &Slots,
    anchors: &[stability::Anchors],
    modes: &[BandMode],
) -> Vec<Columns> {
    let es = p.spacing.edge;
    let mut cols: Vec<Columns> = vec![Columns::default(); bands.len()];
    for b in 0..bands.len() {
        let columns = bands[b].columns();
        let counts = {
            let ctx = Ctx { p, bands: &*bands, item_of, slots, shared: None };
            let (top, bottom) = local_bounds(&bands[b], p, b);
            let segs = routing::channels::collect(&ctx, b, top, bottom, &|other| other > b);
            routing::channels::assign_tracks(&segs, columns + 1).1
        };
        let origin = p.bands[b].insets.left;
        let fresh_x = match modes[b] {
            BandMode::Fresh => {
                coordinates::assign_x(&mut bands[b], &p.spacing, &counts, origin);
                true
            }
            BandMode::Reproduced => {
                coordinates::assign_x(&mut bands[b], &p.spacing, &counts, origin);
                stability::reproduce_x(&mut bands[b], &anchors[b])
            }
            BandMode::Inserted => false,
        };
        if !fresh_x {
            let min_track = (es / 2.0).max(1.0);
            let min_channel: Vec<f32> = counts.iter().map(|&t| (2.0 * es).max((t as f32 + 1.0) * min_track)).collect();
            stability::place_x(&mut bands[b], &anchors[b], &p.spacing, &min_channel);
        }
        let entry = coordinates::channel_width(&p.spacing, counts.first().copied().unwrap_or(0), false);
        let exit = coordinates::channel_width(&p.spacing, counts.get(columns).copied().unwrap_or(0), false);
        cols[b] = Columns::of(&bands[b], entry, exit);
    }
    cols
}

/// How a band is placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BandMode {
    /// No previous positions: laid out from scratch.
    Fresh,
    /// Unchanged since the previous layout: laid out from scratch, which
    /// reproduced the previous positions, and moved back into place.
    Reproduced,
    /// Changed: previous positions kept and the changes fitted into free
    /// space.
    Inserted,
}

/// Layers for every band, and which edges lie on a cycle.
fn layer_bands(p: &Problem<'_>) -> Result<(Vec<u32>, Vec<bool>), LayoutError> {
    let mut layer_of = vec![0u32; p.nodes.len()];
    let mut on_cycle = vec![false; p.edges.len()];
    let mut local = vec![0usize; p.nodes.len()];
    for (b, band) in p.bands.iter().enumerate() {
        for (i, &v) in band.nodes.iter().enumerate() {
            local[v] = i;
        }
        let fixes: Vec<problem::Fix> = band.nodes.iter().map(|&v| p.nodes[v].fix).collect();
        let mut ids = Vec::new();
        let mut edges = Vec::new();
        for (e, edge) in p.edges.iter().enumerate() {
            let (s, t) = (edge.source.node, edge.target.node);
            if s == t || p.nodes[s].band != b || p.nodes[t].band != b {
                continue;
            }
            let labelled = edge.label.is_some() && p.kinds[e] == EdgeKind::Chain;
            ids.push(e);
            edges.push(LayerEdge { source: local[s], target: local[t], minlen: if labelled { 2 } else { 1 } });
        }
        let key = |i: usize| p.key(band.nodes[i]).to_string();
        let layering = layering::assign(&fixes, &edges, &key)?;
        for (i, &v) in band.nodes.iter().enumerate() {
            layer_of[v] = layering.layers[i];
        }
        for (k, &e) in ids.iter().enumerate() {
            on_cycle[e] = layering.on_cycle[k];
        }
    }
    Ok((layer_of, on_cycle))
}

/// A band's cross extent before stacking, with its insets.
fn local_bounds(bg: &BandGraph, p: &Problem<'_>, b: usize) -> (f32, f32) {
    let (lo, hi) = bg
        .items
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), it| (lo.min(it.box_top()), hi.max(it.box_bottom())));
    let insets = p.bands[b].insets;
    if lo.is_finite() { (lo - insets.top, hi + insets.bottom) } else { (0.0, 0.0) }
}

/// Sort keys for unported edge ends, so each side's implicit slots follow
/// where their edges head: by neighbour order in fresh bands, by neighbour
/// height in stable ones.
fn implicit_keys(
    p: &Problem<'_>,
    bands: &[BandGraph],
    chain_of: &[Option<(usize, usize)>],
    stable: &[bool],
) -> Vec<[f64; 2]> {
    let fractions: Vec<Vec<f64>> = bands
        .iter()
        .map(|bg| {
            let mut f = vec![0.5f64; bg.items.len()];
            for layer in &bg.layers {
                for (i, &item) in layer.iter().enumerate() {
                    f[item] = (i as f64 + 0.5) / layer.len() as f64;
                }
            }
            f
        })
        .collect();
    let centre = |b: usize, item: usize| {
        let it = &bands[b].items[item];
        f64::from(it.top + it.height / 2.0)
    };
    p.edges
        .iter()
        .enumerate()
        .map(|(e, edge)| {
            let mut out = [0.0f64; 2];
            for (k, source) in [(0usize, true), (1usize, false)] {
                let here = edge.end(source).node;
                let there = edge.end(!source).node;
                let band = p.nodes[here].band;
                out[k] = match p.kinds[e] {
                    EdgeKind::SelfLoop => -1e12 + 2.0 * e as f64 + k as f64,
                    EdgeKind::Chain => match chain_of[e] {
                        Some((b, c)) => {
                            let items = &bands[b].chains[c].items;
                            let last = items.len() - 1;
                            if stable[b] {
                                centre(b, if source { items[last] } else { items[0] })
                            } else {
                                fractions[b][if source { items[1] } else { items[last - 1] }]
                            }
                        }
                        None => 0.5,
                    },
                    EdgeKind::CrossBand => {
                        if p.nodes[there].band > band {
                            1e9
                        } else {
                            -1e9
                        }
                    }
                    EdgeKind::Pinned => match (stable[band], p.nodes[there].pin) {
                        (true, Some(pin)) => f64::from(pin.y + p.nodes[there].size.height / 2.0),
                        _ => 0.5,
                    },
                };
            }
            out
        })
        .collect()
}

fn assemble(
    ctx: &Ctx<'_, '_>,
    layer_of: &[u32],
    on_cycle: &[bool],
    placement: &bands::Placement,
    routes: routing::Routes,
    label_boxes: Vec<Option<Rect>>,
) -> LayoutResult {
    let p = ctx.p;
    let frame = p.frame;
    let rects: Vec<Rect> = (0..p.nodes.len()).map(|v| ctx.node_rect(v)).collect();

    // Order within each (band, layer), top to bottom.
    let mut keyed: Vec<(usize, u32, f32, f32, usize)> =
        (0..p.nodes.len()).map(|v| (p.nodes[v].band, layer_of[v], rects[v].top(), rects[v].left(), v)).collect();
    keyed.sort_by(|a, b| {
        (a.0, a.1).cmp(&(b.0, b.1)).then(a.2.total_cmp(&b.2)).then(a.3.total_cmp(&b.3)).then(a.4.cmp(&b.4))
    });
    let mut order = vec![0u32; p.nodes.len()];
    let mut i = 0;
    while i < keyed.len() {
        let mut j = i;
        while j < keyed.len() && (keyed[j].0, keyed[j].1) == (keyed[i].0, keyed[i].1) {
            order[keyed[j].4] = u32::try_from(j - i).unwrap_or(u32::MAX);
            j += 1;
        }
        i = j;
    }

    let nodes: Vec<NodePlacement> = (0..p.nodes.len())
        .map(|v| NodePlacement { rect: frame.rect(rects[v]), layer: layer_of[v], order: order[v] })
        .collect();

    let edges: Vec<EdgeRoute> = routes
        .points
        .into_iter()
        .zip(label_boxes)
        .enumerate()
        .map(|(e, (points, label))| {
            let edge = &p.edges[e];
            let (s, t) = (edge.source.node, edge.target.node);
            let same_band = p.nodes[s].band == p.nodes[t].band || p.shared;
            let reversed = s != t && same_band && on_cycle[e] && layer_of[t] <= layer_of[s];
            EdgeRoute {
                points: points.into_iter().map(|pt| frame.point(pt)).collect(),
                reversed,
                label: label.map(|r| frame.rect(r)),
            }
        })
        .collect();

    let groups: Vec<Rect> = (0..p.graph.group_count())
        .map(|g| placement.outer.get(g + 1).copied().flatten().map_or_else(Rect::default, |r| frame.rect(r)))
        .collect();

    let mut bounds: Option<Rect> = None;
    let mut grow = |r: Rect| bounds = Some(bounds.map_or(r, |b| b.union(&r)));
    nodes.iter().for_each(|n| grow(n.rect));
    groups.iter().for_each(|g| grow(*g));
    for route in &edges {
        for pt in &route.points {
            grow(Rect::from_origin_size(*pt, Default::default()));
        }
        if let Some(l) = route.label {
            grow(l);
        }
    }
    LayoutResult::new(nodes, edges, groups, bounds.unwrap_or_default())
}
