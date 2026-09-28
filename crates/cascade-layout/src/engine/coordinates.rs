//! Fresh-layout coordinates.
//!
//! Cross axis (`y`): a priority-style method. Every link between
//! neighbouring columns pulls its two ends into line (attachment point to
//! attachment point, so ports and slots line up), weighted so long edges
//! straighten first: dummy–dummy links weigh 8, node–dummy 2, node–node 1.
//! Columns are solved one at a time, exactly, with the L1 pool-adjacent-
//! violators solver, sweeping back and forth until nothing moves. Pulls
//! from the left column weigh slightly more, which breaks ties toward
//! aligning with predecessors (like Brandes–Köpf's upper-left alignment).
//!
//! Column-by-column descent cannot move a long chain of dummies as a unit,
//! so a straightening pass then moves each chain, whole, onto its source's
//! line (or its target's) wherever every column it crosses has room — the
//! effect of Brandes–Köpf's block alignment. A last greedy pass straightens
//! any remaining bent link by moving one end onto the other's line, when
//! its column has room and none of that end's straight links would bend.
//!
//! Main axis (`x`): columns are as wide as their widest item and centred on
//! a common line; channels between them are `layer_spacing` wide or wider
//! when their tracks need it.

use super::layered::{BandGraph, ItemKind};
use super::packing::{Target, place_l1};
use super::problem::{Problem, Spacing};
use super::slots::Slots;

const SWEEPS: usize = 12;
const LEFT_BIAS: f32 = 1.02;
const STAY_WEIGHT: f32 = 1e-3;

/// A link between neighbouring columns and the offsets of its line at
/// each end.
#[derive(Clone, Copy, Debug)]
struct Link {
    a: usize,
    b: usize,
    off_a: f32,
    off_b: f32,
    weight: f32,
}

/// Pull of item `from` toward alignment with item `other`: `top[from]`
/// wants `top[other] + delta`.
#[derive(Clone, Copy, Debug)]
struct Pull {
    other: usize,
    delta: f32,
    weight: f32,
}

/// Cross offset from an item's top to the line a chain link uses there.
pub(crate) fn link_offset(bg: &BandGraph, problem: &Problem<'_>, slots: &Slots, chain: usize, position: usize) -> f32 {
    let c = &bg.chains[chain];
    let item = &bg.items[c.items[position]];
    match item.kind {
        ItemKind::Node(_) => slots.line_offset(problem, c.edge, position == 0, problem.spacing.edge),
        ItemKind::Dummy { .. } | ItemKind::Label { .. } => item.anchor,
    }
}

/// Minimum distance from item `a`'s top to the next item `b`'s top.
pub(crate) fn min_gap(bg: &BandGraph, problem: &Problem<'_>, a: usize, b: usize) -> f32 {
    let (x, y) = (&bg.items[a], &bg.items[b]);
    x.height + x.margin_bottom + problem.separation(x.is_node(), y.is_node()) + y.margin_top
}

pub(crate) fn assign_y(bg: &mut BandGraph, problem: &Problem<'_>, slots: &Slots) {
    let n = bg.items.len();
    let mut pulls: Vec<Vec<Pull>> = vec![Vec::new(); n];
    let mut links: Vec<Link> = Vec::new();
    for ci in 0..bg.chains.len() {
        let len = bg.chains[ci].items.len();
        for k in 0..len - 1 {
            let (a, b) = (bg.chains[ci].items[k], bg.chains[ci].items[k + 1]);
            let (la, lb) = (bg.items[a].layer, bg.items[b].layer);
            if la.abs_diff(lb) != 1 {
                continue;
            }
            let off_a = link_offset(bg, problem, slots, ci, k);
            let off_b = link_offset(bg, problem, slots, ci, k + 1);
            let weight = match (bg.items[a].is_node(), bg.items[b].is_node()) {
                (true, true) => 1.0,
                (false, false) => 8.0,
                _ => 2.0,
            };
            let bias = |from_layer: usize, other_layer: usize| if other_layer < from_layer { LEFT_BIAS } else { 1.0 };
            pulls[a].push(Pull { other: b, delta: off_b - off_a, weight: weight * bias(la, lb) });
            pulls[b].push(Pull { other: a, delta: off_a - off_b, weight: weight * bias(lb, la) });
            links.push(Link { a, b, off_a, off_b, weight });
        }
    }

    // Start from each column packed and centred on 0.
    for l in 0..bg.columns() {
        let items = bg.layers[l].clone();
        let mut y = 0.0f32;
        for (i, &it) in items.iter().enumerate() {
            if i > 0 {
                y += min_gap(bg, problem, items[i - 1], it);
            }
            bg.items[it].top = y;
        }
        let (first, last) = (items.first().copied(), items.last().copied());
        if let (Some(f), Some(l)) = (first, last) {
            let span = bg.items[l].box_bottom() - bg.items[f].box_top();
            let shift = -bg.items[f].box_top() - span / 2.0;
            for &it in &items {
                bg.items[it].top += shift;
            }
        }
    }

    sweep(bg, problem, &pulls, SWEEPS);
    straighten_chains(bg, problem, slots);
    sweep(bg, problem, &pulls, SWEEPS / 3);
    straighten_chains(bg, problem, slots);
    align_links(bg, problem, &links);

    // Normalise so the band's content starts at 0.
    let min_top = bg.items.iter().map(|i| i.box_top()).fold(f32::INFINITY, f32::min);
    if min_top.is_finite() {
        for it in &mut bg.items {
            it.top -= min_top;
        }
    }
}

fn sweep(bg: &mut BandGraph, problem: &Problem<'_>, pulls: &[Vec<Pull>], rounds: usize) {
    let columns = bg.columns();
    for _ in 0..rounds {
        let mut moved = 0.0f32;
        let order: Vec<usize> = (0..columns).chain((0..columns).rev()).collect();
        for l in order {
            moved = moved.max(solve_column(bg, problem, pulls, l));
        }
        if moved < 0.05 {
            break;
        }
    }
}

/// Move each chain's dummies, together, onto one line: the source's line if
/// every column has room there, else the target's, else the nearest line
/// with room to the source's. Longest chains go first.
fn straighten_chains(bg: &mut BandGraph, problem: &Problem<'_>, slots: &Slots) {
    let mut chains: Vec<usize> = (0..bg.chains.len()).filter(|&c| bg.chains[c].items.len() > 2).collect();
    chains.sort_by_key(|&c| (std::cmp::Reverse(bg.chains[c].items.len()), c));
    let mut position = vec![0usize; bg.items.len()];
    for layer in &bg.layers {
        for (i, &item) in layer.iter().enumerate() {
            position[item] = i;
        }
    }
    for c in chains {
        let len = bg.chains[c].items.len();
        let (mut lo, mut hi) = (f32::NEG_INFINITY, f32::INFINITY);
        for k in 1..len - 1 {
            let d = bg.chains[c].items[k];
            let it = &bg.items[d];
            let layer = &bg.layers[it.layer];
            let at = position[d];
            if at > 0 {
                let prev = &bg.items[layer[at - 1]];
                let sep = problem.separation(prev.is_node(), it.is_node());
                lo = lo.max(prev.box_bottom() + sep + it.margin_top + it.anchor);
            }
            if let Some(&next) = layer.get(at + 1) {
                let next = &bg.items[next];
                let sep = problem.separation(it.is_node(), next.is_node());
                hi = hi.min(next.box_top() - sep - (it.height - it.anchor) - it.margin_bottom);
            }
        }
        if lo > hi {
            continue;
        }
        let source_line = bg.items[bg.chains[c].items[0]].top + link_offset(bg, problem, slots, c, 0);
        let target_line = bg.items[bg.chains[c].items[len - 1]].top + link_offset(bg, problem, slots, c, len - 1);
        let line = if (lo..=hi).contains(&source_line) {
            source_line
        } else if (lo..=hi).contains(&target_line) {
            target_line
        } else {
            source_line.clamp(lo, hi)
        };
        for k in 1..len - 1 {
            let d = bg.chains[c].items[k];
            let anchor = bg.items[d].anchor;
            bg.items[d].top = line - anchor;
        }
    }
}

/// Range of tops an item can take without passing its column neighbours.
fn free_range(bg: &BandGraph, problem: &Problem<'_>, position: &[usize], item: usize) -> (f32, f32) {
    let it = &bg.items[item];
    let layer = &bg.layers[it.layer];
    let at = position[item];
    let lo = match at.checked_sub(1) {
        Some(p) => {
            let prev = &bg.items[layer[p]];
            prev.box_bottom() + problem.separation(prev.is_node(), it.is_node()) + it.margin_top
        }
        None => f32::NEG_INFINITY,
    };
    let hi = match layer.get(at + 1) {
        Some(&n) => {
            let next = &bg.items[n];
            next.box_top() - problem.separation(it.is_node(), next.is_node()) - it.height - it.margin_bottom
        }
        None => f32::INFINITY,
    };
    (lo, hi)
}

/// Straighten bent links greedily: move one end onto the other's line when
/// its column has room and none of its straight links would bend. Every
/// move adds a straight link and removes none, so this terminates.
fn align_links(bg: &mut BandGraph, problem: &Problem<'_>, links: &[Link]) {
    const STRAIGHT: f32 = 0.01;
    let mut position = vec![0usize; bg.items.len()];
    for layer in &bg.layers {
        for (i, &item) in layer.iter().enumerate() {
            position[item] = i;
        }
    }
    let mut of_item: Vec<Vec<usize>> = vec![Vec::new(); bg.items.len()];
    for (i, l) in links.iter().enumerate() {
        of_item[l.a].push(i);
        of_item[l.b].push(i);
    }
    let bend = |bg: &BandGraph, l: &Link| (bg.items[l.a].top + l.off_a - bg.items[l.b].top - l.off_b).abs();
    let mut order: Vec<usize> = (0..links.len()).collect();
    order.sort_by(|&x, &y| links[y].weight.total_cmp(&links[x].weight).then(x.cmp(&y)));
    for _ in 0..4 {
        let mut changed = false;
        for &li in &order {
            let l = links[li];
            if bend(bg, &l) < STRAIGHT {
                continue;
            }
            for (mover, top) in
                [(l.b, bg.items[l.a].top + l.off_a - l.off_b), (l.a, bg.items[l.b].top + l.off_b - l.off_a)]
            {
                let (lo, hi) = free_range(bg, problem, &position, mover);
                if top < lo || top > hi {
                    continue;
                }
                let keeps_straight = of_item[mover].iter().all(|&o| o == li || bend(bg, &links[o]) >= STRAIGHT);
                if keeps_straight {
                    bg.items[mover].top = top;
                    changed = true;
                    break;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

fn solve_column(bg: &mut BandGraph, problem: &Problem<'_>, pulls: &[Vec<Pull>], l: usize) -> f32 {
    let items = &bg.layers[l];
    if items.is_empty() {
        return 0.0;
    }
    let mut gaps = Vec::with_capacity(items.len());
    let mut targets = Vec::with_capacity(items.len());
    let mut current = Vec::with_capacity(items.len());
    for (i, &it) in items.iter().enumerate() {
        gaps.push(if i == 0 { 0.0 } else { min_gap(bg, problem, items[i - 1], it) });
        let top = bg.items[it].top;
        let mut t: Vec<Target> =
            pulls[it].iter().map(|p| Target { value: bg.items[p.other].top + p.delta, weight: p.weight }).collect();
        t.push(Target { value: top, weight: STAY_WEIGHT });
        targets.push(t);
        current.push(top);
    }
    let tops = place_l1(&gaps, &targets, &current);
    let mut moved = 0.0f32;
    for (&it, &top) in items.iter().zip(&tops) {
        moved = moved.max((bg.items[it].top - top).abs());
        bg.items[it].top = top;
    }
    moved
}

/// Column widths: every item's reserved main-axis room.
pub(crate) fn column_widths(bg: &BandGraph) -> Vec<f32> {
    bg.layers.iter().map(|items| items.iter().map(|&i| bg.items[i].reserve).fold(0.0, f32::max)).collect()
}

/// Width of a channel holding `tracks` parallel segments. Inner channels
/// are at least `layer_spacing` wide; the outer ones (0 and `columns`) only
/// exist when used.
pub(crate) fn channel_width(spacing: &Spacing, tracks: usize, inner: bool) -> f32 {
    let needed = (tracks as f32 + 1.0) * spacing.edge;
    if inner {
        spacing.layer.max(needed)
    } else if tracks == 0 {
        0.0
    } else {
        needed
    }
}

/// Place columns left to right from `origin`, centring items in their
/// column.
pub(crate) fn assign_x(bg: &mut BandGraph, spacing: &Spacing, tracks: &[usize], origin: f32) {
    let widths = column_widths(bg);
    let columns = bg.columns();
    let mut x = origin + channel_width(spacing, tracks.first().copied().unwrap_or(0), false);
    for (l, &w) in widths.iter().enumerate() {
        for &it in &bg.layers[l] {
            let item = &mut bg.items[it];
            item.x = x + (w - item.width) / 2.0;
        }
        x += w;
        if l + 1 < columns {
            x += channel_width(spacing, tracks.get(l + 1).copied().unwrap_or(0), true);
        }
    }
}
