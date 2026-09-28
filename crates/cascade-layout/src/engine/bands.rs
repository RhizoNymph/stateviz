//! Stacking bands (groups) top to bottom, and pushing items off pins.
//!
//! The ungrouped band (when it has nodes) comes first, then every group in
//! insertion order. A fresh band is placed right below the previous one's
//! gap, sized for its cross-band tracks plus a spare. A band with previous
//! positions stays put and only moves down if the gap above would close to
//! less than half the group spacing, or its tracks no longer fit at half
//! the edge spacing, so a change in one band does not move the others.
//! Each band's items are pushed off
//! pinned nodes before the band's rect is measured, so everything below
//! makes room. Group rects contain their contents (and pinned members) plus
//! padding and header, and all groups share one width, like swimlanes.

use crate::geometry::Rect;

use super::context::Columns;
use super::layered::BandGraph;
use super::packing::{Stacked, push_off};
use super::problem::Problem;

#[derive(Clone, Debug)]
pub(crate) struct Placement {
    /// Outer rect of every stacked band (group rect for groups).
    pub outer: Vec<Option<Rect>>,
    /// Stacked bands, top to bottom.
    pub order: Vec<usize>,
    /// (top, bottom) of the gap after each stacked band but the last.
    pub gaps: Vec<(f32, f32)>,
}

/// The band's items and channels, if it has any.
pub(crate) fn content_box(bg: &BandGraph, cols: &Columns) -> Option<Rect> {
    let (x0, x1) = cols.span()?;
    let (y0, y1) = bg
        .items
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), it| (lo.min(it.box_top()), hi.max(it.box_bottom())));
    y0.is_finite().then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
}

/// Bands that take part in stacking.
pub(crate) fn stacked_bands(problem: &Problem<'_>) -> Vec<usize> {
    (0..problem.bands.len()).filter(|&b| b > 0 || !problem.bands[0].nodes.is_empty()).collect()
}

fn resolve_pins(bg: &mut BandGraph, cols: &Columns, problem: &Problem<'_>, pinned: &[Rect]) {
    if pinned.is_empty() {
        return;
    }
    for l in 0..bg.columns() {
        let (xl, xr) = (cols.left[l], cols.right[l]);
        let obstacles: Vec<(f32, f32)> =
            pinned.iter().filter(|r| r.left() < xr && r.right() > xl).map(|r| (r.top(), r.bottom())).collect();
        if obstacles.is_empty() {
            continue;
        }
        let items = &bg.layers[l];
        let stacked: Vec<Stacked> = items
            .iter()
            .enumerate()
            .map(|(k, &i)| {
                let it = &bg.items[i];
                let gap = if k == 0 { 0.0 } else { problem.separation(bg.items[items[k - 1]].is_node(), it.is_node()) };
                Stacked {
                    top: it.top,
                    above: it.margin_top,
                    below: it.height + it.margin_bottom,
                    gap,
                    clearance: problem.separation(it.is_node(), true),
                }
            })
            .collect();
        let tops = push_off(&stacked, &obstacles);
        for (&i, top) in items.iter().zip(tops) {
            bg.items[i].top = top;
        }
    }
}

/// Height a fresh layout gives the gap holding `tracks` cross-band runs:
/// room for them plus one spare, so a later edit adding one usually fits.
fn fresh_gap(problem: &Problem<'_>, tracks: usize) -> f32 {
    problem.spacing.group.max((tracks as f32 + 2.0) * problem.spacing.edge)
}

/// Smallest gap a kept band tolerates above it before it moves: half the
/// group spacing, with tracks allowed to close up to half the edge spacing.
fn kept_gap(problem: &Problem<'_>, tracks: usize) -> f32 {
    (problem.spacing.group / 2.0).max((tracks as f32 + 1.0) * problem.spacing.edge / 2.0)
}

/// Stack the bands. `kept[b]` keeps band `b` where its items already are
/// (it only moves down if the band above now reaches into it);
/// `gap_tracks[k]` is the number of cross-band tracks in the gap after the
/// `k`-th stacked band.
pub(crate) fn stack(
    problem: &Problem<'_>,
    bands: &mut [BandGraph],
    cols: &[Columns],
    kept: &[bool],
    gap_tracks: &[usize],
) -> Placement {
    let pinned: Vec<(usize, Rect)> = problem
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(v, n)| n.pin.map(|pin| (v, Rect::from_origin_size(pin, n.size))))
        .collect();
    let pinned_rects: Vec<Rect> = pinned.iter().map(|(_, r)| *r).collect();
    let order = stacked_bands(problem);
    let mut outer: Vec<Option<Rect>> = vec![None; bands.len()];
    // Bottom of the band above and the tracks in the gap below it.
    let mut above: Option<(f32, usize)> = None;
    for (k, &b) in order.iter().enumerate() {
        let insets = problem.bands[b].insets;
        let cursor = above.map(|(bottom, tracks)| {
            bottom + if kept[b] { kept_gap(problem, tracks) } else { fresh_gap(problem, tracks) }
        });
        if let Some(content) = content_box(&bands[b], &cols[b]) {
            let top = content.top() - insets.top;
            let dy = match (cursor, kept[b]) {
                (None, false) => -top,
                (None, true) => 0.0,
                (Some(c), false) => c - top,
                (Some(c), true) if c - top > super::stability::TOLERANCE => c - top,
                (Some(_), true) => 0.0,
            };
            for it in &mut bands[b].items {
                it.top += dy;
            }
        }
        resolve_pins(&mut bands[b], &cols[b], problem, &pinned_rects);
        let mut rect = content_box(&bands[b], &cols[b]).map(|c| c.outset(insets));
        for (v, r) in &pinned {
            if problem.nodes[*v].band == b {
                let r = r.outset(insets);
                rect = Some(rect.map_or(r, |x| x.union(&r)));
            }
        }
        let rect = rect.unwrap_or_else(|| {
            Rect::new(0.0, cursor.unwrap_or(0.0), insets.left + insets.right, insets.top + insets.bottom)
        });
        outer[b] = Some(rect);
        above = Some((rect.bottom(), gap_tracks.get(k).copied().unwrap_or(0)));
    }

    // Groups share one width.
    let groups: Vec<usize> = order.iter().copied().filter(|&b| problem.bands[b].group.is_some()).collect();
    let span = groups.iter().filter_map(|&b| outer[b]).fold(None, |acc: Option<(f32, f32)>, r| {
        Some(acc.map_or((r.left(), r.right()), |(l, h)| (l.min(r.left()), h.max(r.right()))))
    });
    if let Some((l, r)) = span {
        for &b in &groups {
            if let Some(rect) = outer[b].as_mut() {
                *rect = Rect::new(l, rect.top(), r - l, rect.size.height);
            }
        }
    }

    let gaps = order
        .windows(2)
        .map(|w| {
            let above = outer[w[0]].map_or(0.0, |r| r.bottom());
            let below = outer[w[1]].map_or(above, |r| r.top());
            (above, below.max(above))
        })
        .collect();
    Placement { outer, order, gaps }
}
