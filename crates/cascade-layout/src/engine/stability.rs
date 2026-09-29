//! Stable-mode placement: keep the previous layout and fit changes into it.
//!
//! A node is *anchored* when the previous layout had it in the same layer
//! (and near its column). Anchored nodes keep their previous position
//! exactly; they only move when they would really collide (a node grew by
//! more than half the node spacing, or new stubs would run into the node
//! below) or a column has to make room (a new wide node). Everything else is
//! placed into free space without moving anything:
//!
//! 1. new nodes as close as possible to the average height of their placed
//!    neighbours (propagated outward from anchored nodes);
//! 2. each edge's dummies on one line clear through all their columns if
//!    such a line exists near the edge's source (or target), so long edges
//!    stay straight; otherwise each dummy at the nearest free spot;
//! 3. new columns next to their neighbours, and any column pushed right
//!    only as far as needed to keep its channel open.
//!
//! A band whose contents did not change is not placed this way at all: a
//! fresh layout of it reproduces the previous node positions up to one
//! translation, and [`reproduce_y`]/[`reproduce_x`] move that fresh layout
//! back into place, so its dummies, routes and rect come back exactly too.

use crate::geometry::Point;

use super::coordinates::link_offset;
use super::layered::{BandGraph, ItemId};
use super::packing::Occupancy;
use super::problem::{Problem, Spacing};
use super::slots::Slots;

/// Per item: the previous canonical top-left of an anchored node.
pub(crate) type Anchors = Vec<Option<Point>>;

/// Rounding slack when checking whether a kept position still fits:
/// previous positions are reused bit for bit unless they are really off.
pub(crate) const TOLERANCE: f32 = 0.01;

/// Decide which of the band's nodes are anchored.
pub(crate) fn anchors(bg: &BandGraph, problem: &Problem<'_>, layer_of: &[u32]) -> Anchors {
    let mut anchors: Anchors = bg
        .items
        .iter()
        .map(|item| {
            let n = item.node()?;
            let prev = problem.nodes[n].prev?;
            (prev.layer == layer_of[n]).then_some(prev.origin)
        })
        .collect();
    // An anchored node far from its column (it used to be pinned elsewhere)
    // is placed afresh instead of stretching the column.
    for layer in &bg.layers {
        let mut centres: Vec<f32> =
            layer.iter().filter_map(|&i| anchors[i].map(|p| p.x + bg.items[i].width / 2.0)).collect();
        if centres.is_empty() {
            continue;
        }
        centres.sort_by(f32::total_cmp);
        let median = centres[centres.len() / 2];
        let reach = layer.iter().map(|&i| bg.items[i].reserve).fold(0.0f32, f32::max) / 2.0 + 1.0;
        for &i in layer {
            if let Some(p) = anchors[i]
                && (p.x + bg.items[i].width / 2.0 - median).abs() > reach
            {
                anchors[i] = None;
            }
        }
    }
    anchors
}

/// The one translation that maps every node item onto its previous
/// position along an axis, if all node items are anchored and agree.
fn common_shift(bg: &BandGraph, anchors: &Anchors, delta: impl Fn(f32, f32, Point) -> f32) -> Option<f32> {
    let mut shift: Option<f32> = None;
    for (i, it) in bg.items.iter().enumerate() {
        if !it.is_node() {
            continue;
        }
        let d = delta(it.x, it.top, anchors[i]?);
        match shift {
            None => shift = Some(d),
            Some(s) if (s - d).abs() <= TOLERANCE => {}
            Some(_) => return None,
        }
    }
    shift
}

/// If a fresh placement of the band put every node at one vertical
/// translation of its previous position (the band did not change), move it
/// back: nodes exactly onto their previous tops, everything else by the
/// same translation. Returns whether it did.
pub(crate) fn reproduce_y(bg: &mut BandGraph, anchors: &Anchors) -> bool {
    let Some(dy) = common_shift(bg, anchors, |_, top, prev| prev.y - top) else { return false };
    for (it, anchor) in bg.items.iter_mut().zip(anchors) {
        it.top = anchor.map_or(it.top + dy, |p| p.y);
    }
    true
}

/// The same along the main axis, after the fresh column placement. A band
/// laid out with [`super::align`] moved its columns separately, so besides
/// one translation for the whole band this accepts one per column: every
/// column with nodes has all of them anchored at one translation, a column
/// of dummies and labels only moves with the column before it (as the
/// alignment moves it), and no channel ends up narrower than the fresh
/// placement made it.
pub(crate) fn reproduce_x(bg: &mut BandGraph, anchors: &Anchors) -> bool {
    if let Some(dx) = common_shift(bg, anchors, |x, _, prev| prev.x - x) {
        for (it, anchor) in bg.items.iter_mut().zip(anchors) {
            it.x = anchor.map_or(it.x + dx, |p| p.x);
        }
        return true;
    }
    let columns = bg.columns();
    let mut shift = vec![0.0f32; columns];
    for l in 0..columns {
        let mut column: Option<f32> = None;
        for &i in &bg.layers[l] {
            let it = &bg.items[i];
            if !it.is_node() {
                continue;
            }
            let Some(prev) = anchors[i] else { return false };
            let d = prev.x - it.x;
            match column {
                None => column = Some(d),
                Some(c) if (c - d).abs() <= TOLERANCE => {}
                Some(_) => return false,
            }
        }
        shift[l] = match (column, l.checked_sub(1)) {
            (Some(d), _) => d,
            (None, Some(before)) => shift[before],
            (None, None) => return false,
        };
    }
    let before = column_extents(bg);
    for l in 1..columns {
        let fresh = before[l].0 - before[l - 1].1;
        let kept = (before[l].0 + shift[l]) - (before[l - 1].1 + shift[l - 1]);
        if kept + TOLERANCE < fresh {
            return false;
        }
    }
    for (l, layer) in bg.layers.iter().enumerate() {
        for &i in layer {
            let it = &mut bg.items[i];
            it.x = anchors[i].map_or(it.x + shift[l], |p| p.x);
        }
    }
    true
}

/// Main-axis extent (left, right) of every column's reserved room.
fn column_extents(bg: &BandGraph) -> Vec<(f32, f32)> {
    bg.layers
        .iter()
        .map(|layer| {
            layer.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &i| {
                let it = &bg.items[i];
                let cx = it.x + it.width / 2.0;
                (lo.min(cx - it.reserve / 2.0), hi.max(cx + it.reserve / 2.0))
            })
        })
        .collect()
}

fn separation<'a>(problem: &'a Problem<'_>, is_node: bool) -> impl Fn(bool) -> f32 + 'a {
    move |other_is_node| problem.separation(is_node, other_is_node)
}

/// Phase 1: anchored nodes at their previous positions, new nodes near
/// their neighbours. Returns each column's occupancy for phase 2.
pub(crate) fn place_nodes(bg: &mut BandGraph, problem: &Problem<'_>, anchors: &Anchors) -> Vec<Occupancy> {
    let columns = bg.columns();
    let mut occupancy = vec![Occupancy::default(); columns];
    let mut placed = vec![false; bg.items.len()];

    for (l, column) in occupancy.iter_mut().enumerate() {
        let mut anchored: Vec<ItemId> = bg.layers[l].iter().copied().filter(|&i| anchors[i].is_some()).collect();
        anchored.sort_by(|&a, &b| {
            let order = |i: ItemId| bg.items[i].node().and_then(|n| problem.nodes[n].prev).map_or(0, |p| p.order);
            let y = |i: ItemId| anchors[i].map_or(0.0, |p| p.y);
            order(a).cmp(&order(b)).then(y(a).total_cmp(&y(b))).then(a.cmp(&b))
        });
        // Kept nodes only move apart when they would really collide: their
        // stub room (margins) plus half an edge spacing, and at least half
        // the node spacing between them. New stubs on one node therefore
        // do not push its neighbour away.
        let mut above: Option<ItemId> = None;
        for i in anchored {
            let Some(prev) = anchors[i] else { continue };
            let lowest = above.map_or(f32::NEG_INFINITY, |q| {
                let (q, it) = (&bg.items[q], &bg.items[i]);
                let clearance =
                    (problem.spacing.node / 2.0).max(q.margin_bottom + it.margin_top + problem.spacing.edge / 2.0);
                q.top + q.height + clearance
            });
            let item = &mut bg.items[i];
            item.top = if prev.y + TOLERANCE >= lowest { prev.y } else { lowest };
            column.insert(item.box_top(), item.box_bottom(), true);
            placed[i] = true;
            above = Some(i);
        }
    }

    // Desired heights for new nodes, spreading out from placed ones along
    // chain ends.
    let mut neighbours: Vec<Vec<ItemId>> = vec![Vec::new(); bg.items.len()];
    for chain in &bg.chains {
        let (s, t) = (chain.items[0], chain.items[chain.items.len() - 1]);
        neighbours[s].push(t);
        neighbours[t].push(s);
    }
    let mut centre: Vec<Option<f32>> =
        (0..bg.items.len()).map(|i| placed[i].then(|| bg.items[i].top + bg.items[i].height / 2.0)).collect();
    let new_nodes: Vec<ItemId> = (0..bg.items.len()).filter(|&i| bg.items[i].is_node() && !placed[i]).collect();
    loop {
        let mut progress = false;
        for &i in &new_nodes {
            if centre[i].is_some() {
                continue;
            }
            let known: Vec<f32> = neighbours[i].iter().filter_map(|&j| centre[j]).collect();
            if !known.is_empty() {
                centre[i] = Some(known.iter().sum::<f32>() / known.len() as f32);
                progress = true;
            }
        }
        if !progress {
            break;
        }
    }
    // A new node with nothing to go by takes its column's first free slot
    // from the band's top: the top of an empty column, or a hole between
    // placed nodes, before the space below them.
    let top = (0..bg.items.len()).filter(|&i| placed[i]).map(|i| bg.items[i].box_top()).fold(f32::INFINITY, f32::min);
    let top = if top.is_finite() { top } else { 0.0 };
    for &i in &new_nodes {
        let item = &bg.items[i];
        let desired_top = match (centre[i], item.node().and_then(|n| problem.nodes[n].prev)) {
            (Some(c), _) => c - item.height / 2.0,
            (None, Some(prev)) => prev.origin.y,
            (None, None) => top + item.margin_top,
        };
        let l = item.layer;
        let sep = separation(problem, true);
        let box_top = if centre[i].is_none() && item.node().and_then(|n| problem.nodes[n].prev).is_none() {
            occupancy[l].first_free(desired_top - item.margin_top, item.extent(), &sep)
        } else {
            occupancy[l].nearest_free(desired_top - item.margin_top, item.extent(), &sep)
        };
        let item = &mut bg.items[i];
        item.top = box_top + item.margin_top;
        occupancy[l].insert(item.box_top(), item.box_bottom(), true);
    }
    occupancy
}

/// Phase 2 (after slots are ordered): each chain's dummies.
pub(crate) fn place_dummies(bg: &mut BandGraph, problem: &Problem<'_>, slots: &Slots, occupancy: &mut [Occupancy]) {
    for ci in 0..bg.chains.len() {
        let len = bg.chains[ci].items.len();
        if len <= 2 {
            continue;
        }
        let dummies: Vec<ItemId> = bg.chains[ci].items[1..len - 1].to_vec();
        let (s, t) = (bg.chains[ci].items[0], bg.chains[ci].items[len - 1]);
        let from_source = bg.items[s].top + link_offset(bg, problem, slots, ci, 0);
        let from_target = bg.items[t].top + link_offset(bg, problem, slots, ci, len - 1);
        let sep = separation(problem, false);
        let fits = |line: f32, occupancy: &[Occupancy]| {
            dummies.iter().all(|&d| {
                let it = &bg.items[d];
                occupancy[it.layer].fits(line - it.anchor - it.margin_top, it.extent(), &sep)
            })
        };
        let mut line = None;
        for want in [from_source, from_target] {
            if fits(want, occupancy) {
                line = Some(want);
                break;
            }
        }
        if line.is_none() {
            let mut candidates: Vec<f32> = dummies
                .iter()
                .map(|&d| {
                    let it = &bg.items[d];
                    occupancy[it.layer].nearest_free(from_source - it.anchor - it.margin_top, it.extent(), &sep)
                        + it.anchor
                        + it.margin_top
                })
                .collect();
            candidates.sort_by(|a, b| (a - from_source).abs().total_cmp(&(b - from_source).abs()));
            line = candidates.into_iter().find(|&c| fits(c, occupancy));
        }
        for &d in &dummies {
            let (layer, anchor, margin_top, extent) = {
                let it = &bg.items[d];
                (it.layer, it.anchor, it.margin_top, it.extent())
            };
            let top = match line {
                Some(l) => l - anchor,
                None => occupancy[layer].nearest_free(from_source - anchor - margin_top, extent, &sep) + margin_top,
            };
            let it = &mut bg.items[d];
            it.top = top;
            occupancy[layer].insert(it.box_top(), it.box_bottom(), false);
        }
    }
    for layer in &mut bg.layers {
        layer.sort_by(|&a, &b| bg.items[a].top.total_cmp(&bg.items[b].top).then(a.cmp(&b)));
    }
}

/// Main-axis positions: anchored nodes keep theirs, everything else is
/// centred on its column, new columns go next to their neighbours, and a
/// column moves right only when its channel (`min_channel[l]`, the space
/// left of column `l`) would be too narrow.
pub(crate) fn place_x(bg: &mut BandGraph, anchors: &Anchors, spacing: &Spacing, min_channel: &[f32]) {
    let columns = bg.columns();
    let widths: Vec<f32> =
        bg.layers.iter().map(|l| l.iter().map(|&i| bg.items[i].reserve).fold(0.0, f32::max)).collect();
    let mut centre: Vec<Option<f32>> = bg
        .layers
        .iter()
        .map(|layer| {
            let mut c: Vec<f32> =
                layer.iter().filter_map(|&i| anchors[i].map(|p| p.x + bg.items[i].width / 2.0)).collect();
            c.sort_by(f32::total_cmp);
            c.get(c.len() / 2).copied()
        })
        .collect();
    let first_known = centre.iter().position(Option::is_some);
    if let Some(k) = first_known {
        for l in k + 1..columns {
            if centre[l].is_none()
                && let Some(prev) = centre[l - 1]
            {
                centre[l] = Some(prev + widths[l - 1] / 2.0 + spacing.layer + widths[l] / 2.0);
            }
        }
        for l in (0..k).rev() {
            if let Some(next) = centre[l + 1] {
                centre[l] = Some(next - widths[l + 1] / 2.0 - spacing.layer - widths[l] / 2.0);
            }
        }
    }
    for (l, c) in centre.iter().enumerate() {
        let c = c.unwrap_or(0.0);
        for &i in &bg.layers[l] {
            let item = &mut bg.items[i];
            item.x = match anchors[i] {
                Some(p) => p.x,
                None => c - item.width / 2.0,
            };
        }
    }
    let extent = |bg: &BandGraph, l: usize| -> (f32, f32) {
        bg.layers[l].iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &i| {
            let it = &bg.items[i];
            let cx = it.x + it.width / 2.0;
            (lo.min(cx - it.reserve / 2.0), hi.max(cx + it.reserve / 2.0))
        })
    };
    for l in 1..columns {
        let (_, right) = extent(bg, l - 1);
        let (left, _) = extent(bg, l);
        let need = min_channel.get(l).copied().unwrap_or(spacing.edge * 2.0);
        if left + TOLERANCE < right + need {
            let shift = right + need - left;
            for &i in &bg.layers[l] {
                bg.items[i].x += shift;
            }
        }
    }
}
