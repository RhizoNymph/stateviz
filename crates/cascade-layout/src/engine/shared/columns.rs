//! Global columns and where they sit along the main axis.
//!
//! 1. Every distinct layer of any band is a global column, as wide as its
//!    widest item (node, label or edge bend) in any band.
//! 2. Every global channel's zones get as many slots as the band needing
//!    the most tracks there; edges passing bands in between get pass slots
//!    by interval colouring over the gaps they span, so two verticals share
//!    a slot only when their stretches of the stack do not meet.
//! 3. A channel is at least `layer_spacing` wide and holds its slots an
//!    edge spacing apart (the outer channels only exist when used).
//! 4. Column positions: packed left to right from scratch; or, with a
//!    previous layout, a fresh placement that puts every kept node back
//!    where it was is used as is; otherwise each column keeps the centre of
//!    its kept nodes, new columns go next to their neighbours, and a column
//!    moves right only where its channel would get too narrow.
//! 5. Every item is centred on its column; a kept node keeps its previous
//!    position, moved only with its column.

use std::collections::BTreeMap;

use super::super::bands::stacked_bands;
use super::super::context::{Columns, Ctx};
use super::super::layered::BandGraph;
use super::super::local_bounds;
use super::super::problem::{EdgeKind, Problem};
use super::super::routing::channels::{self, cross_channel};
use super::super::slots::Slots;
use super::super::stability::{Anchors, TOLERANCE};
use super::{Caps, Shared, Zone, channel_span, zone_tracks};

/// Place every band's items on the shared columns. Returns the shared
/// geometry and each band's columns.
pub(crate) fn place(
    p: &Problem<'_>,
    bands: &mut [BandGraph],
    item_of: &[Option<usize>],
    slots: &Slots,
    anchors: &[Anchors],
) -> (Shared, Vec<Columns>) {
    let mut values: Vec<u32> = bands.iter().flat_map(|bg| bg.values.iter().copied()).collect();
    values.sort_unstable();
    values.dedup();
    let global_of: Vec<Vec<usize>> =
        bands.iter().map(|bg| bg.values.iter().map(|v| values.binary_search(v).unwrap_or(0)).collect()).collect();
    let n = values.len();

    let mut width = vec![0.0f32; n];
    for (b, bg) in bands.iter().enumerate() {
        for (l, layer) in bg.layers.iter().enumerate() {
            let g = global_of[b][l];
            for &i in layer {
                width[g] = width[g].max(bg.items[i].reserve);
            }
        }
    }

    let mut caps = vec![Caps::default(); n + 1];
    {
        let ctx = Ctx { p, bands: &*bands, item_of, slots, shared: None };
        for (b, bg) in bands.iter().enumerate() {
            let (top, bottom) = local_bounds(bg, p, b);
            let segs = channels::collect(&ctx, b, top, bottom, &|other| other > b);
            for ((g, zone), count) in zone_tracks(&global_of[b], &segs).counts {
                if let Some(cap) = caps.get_mut(g) {
                    cap.raise(zone, count);
                }
            }
        }
    }
    let through = reserve_passes(p, bands, item_of, &global_of, &mut caps);

    let es = p.spacing.edge;
    let full = |g: usize| {
        let need = (caps[g].total() as f32 + 1.0) * es;
        if g == 0 || g == n { if caps[g].total() > 0 { need } else { 0.0 } } else { p.spacing.layer.max(need) }
    };
    let relaxed = |g: usize| (2.0 * es).max((caps[g].total() as f32 + 1.0) * es / 2.0);
    let channel_width: Vec<f32> = (0..=n).map(full).collect();

    let origin = (0..bands.len()).map(|b| p.bands[b].insets.left).fold(0.0f32, f32::max);
    let mut fresh_left = Vec::with_capacity(n);
    let mut x = origin + channel_width[0];
    for g in 0..n {
        fresh_left.push(x);
        x += width[g];
        if g + 1 < n {
            x += channel_width[g + 1];
        }
    }

    // Kept nodes: (band, item, global column, previous x).
    let mut kept: Vec<(usize, usize, usize, f32)> = Vec::new();
    for (b, bg) in bands.iter().enumerate() {
        for (l, layer) in bg.layers.iter().enumerate() {
            for &i in layer {
                if let Some(prev) = anchors.get(b).and_then(|a| a.get(i)).copied().flatten() {
                    kept.push((b, i, global_of[b][l], prev.x));
                }
            }
        }
    }
    let fresh_x = |g: usize, w: f32| fresh_left[g] + (width[g] - w) / 2.0;
    let reproduced = kept.iter().all(|&(b, i, g, px)| (fresh_x(g, bands[b].items[i].width) - px).abs() <= TOLERANCE);

    let (centre, shift) = if reproduced {
        ((0..n).map(|g| fresh_left[g] + width[g] / 2.0).collect(), vec![0.0f32; n])
    } else {
        kept_centres(&kept, bands, &width, &channel_width, &relaxed)
    };

    for (b, bg) in bands.iter_mut().enumerate() {
        let BandGraph { layers, items, .. } = bg;
        for (layer, &g) in layers.iter().zip(&global_of[b]) {
            for &i in layer {
                let prev = anchors.get(b).and_then(|a| a.get(i)).copied().flatten();
                let item = &mut items[i];
                item.x = match prev {
                    Some(prev) if shift[g] == 0.0 => prev.x,
                    Some(prev) => prev.x + shift[g],
                    None if reproduced => fresh_x(g, item.width),
                    None => centre[g] - item.width / 2.0,
                };
            }
        }
    }

    let left: Vec<f32> = (0..n).map(|g| centre[g] - width[g] / 2.0).collect();
    let right: Vec<f32> = (0..n).map(|g| centre[g] + width[g] / 2.0).collect();
    let channels: Vec<(f32, f32)> = if n == 0 {
        vec![(origin, origin)]
    } else {
        (0..=n)
            .map(|g| match g {
                0 => (left[0] - channel_width[0], left[0]),
                g if g == n => (right[n - 1], right[n - 1] + channel_width[n]),
                g => (right[g - 1], left[g]),
            })
            .collect()
    };

    let cols = bands
        .iter()
        .enumerate()
        .map(|(b, bg)| {
            let local = Columns::of(bg, 0.0, 0.0);
            let count = local.count();
            if count == 0 {
                return local;
            }
            let (first, _) = channel_span(&global_of[b], 0);
            let (_, last) = channel_span(&global_of[b], count);
            let entry = channels.get(first).map_or(0.0, |c| (local.left[0] - c.0).max(0.0));
            let exit = channels.get(last).map_or(0.0, |c| (c.1 - local.right[count - 1]).max(0.0));
            Columns::of(bg, entry, exit)
        })
        .collect();

    (Shared { global_of, channels, caps, through }, cols)
}

/// Column centres from the kept nodes, and how far each column moved to
/// keep its channel open.
fn kept_centres(
    kept: &[(usize, usize, usize, f32)],
    bands: &[BandGraph],
    width: &[f32],
    channel_width: &[f32],
    relaxed: &dyn Fn(usize) -> f32,
) -> (Vec<f32>, Vec<f32>) {
    let n = width.len();
    let mut found: Vec<Vec<f32>> = vec![Vec::new(); n];
    for &(b, i, g, px) in kept {
        found[g].push(px + bands[b].items[i].width / 2.0);
    }
    let mut centre: Vec<Option<f32>> = found
        .into_iter()
        .map(|mut c| {
            c.sort_by(f32::total_cmp);
            c.get(c.len() / 2).copied()
        })
        .collect();
    if let Some(k) = centre.iter().position(Option::is_some) {
        for g in k + 1..n {
            if centre[g].is_none()
                && let Some(before) = centre[g - 1]
            {
                centre[g] = Some(before + width[g - 1] / 2.0 + channel_width[g] + width[g] / 2.0);
            }
        }
        for g in (0..k).rev() {
            if let Some(after) = centre[g + 1] {
                centre[g] = Some(after - width[g + 1] / 2.0 - channel_width[g + 1] - width[g] / 2.0);
            }
        }
    }
    let mut centre: Vec<f32> = centre.into_iter().map(|c| c.unwrap_or(0.0)).collect();
    let mut shift = vec![0.0f32; n];
    for g in 1..n {
        let right = centre[g - 1] + width[g - 1] / 2.0;
        let left = centre[g] - width[g] / 2.0;
        let need = relaxed(g);
        if left + TOLERANCE < right + need {
            let s = right + need - left;
            centre[g] += s;
            shift[g] = s;
        }
    }
    (centre, shift)
}

/// Reserve a pass slot for every edge between bands that are not
/// neighbours in the stack, in the global channel right of its source
/// column. Returns edge → (channel, slot) and raises the pass zones.
fn reserve_passes(
    p: &Problem<'_>,
    bands: &[BandGraph],
    item_of: &[Option<usize>],
    global_of: &[Vec<usize>],
    caps: &mut [Caps],
) -> BTreeMap<usize, (usize, usize)> {
    let mut position = vec![None; p.bands.len()];
    for (k, b) in stacked_bands(p).into_iter().enumerate() {
        position[b] = Some(k);
    }
    // (channel, first gap, last gap, edge): the gaps the vertical spans.
    let mut wanted: Vec<(usize, usize, usize, usize)> = Vec::new();
    for (e, edge) in p.edges.iter().enumerate() {
        if p.kinds[e] != EdgeKind::CrossBand {
            continue;
        }
        let (sb, tb) = (p.nodes[edge.source.node].band, p.nodes[edge.target.node].band);
        let (Some(s), Some(t)) = (position[sb], position[tb]) else { continue };
        if s.abs_diff(t) < 2 {
            continue;
        }
        let Some(item) = item_of[edge.source.node] else { continue };
        let column = bands[sb].items[item].layer;
        let (g, _) = channel_span(&global_of[sb], cross_channel(edge.source.side, column, true));
        wanted.push((g, s.min(t), s.max(t) - 1, e));
    }
    wanted.sort_unstable();
    let mut through = BTreeMap::new();
    let mut ends: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (g, first, last, e) in wanted {
        let slots = ends.entry(g).or_default();
        let slot = match slots.iter().position(|&end| end < first) {
            Some(k) => k,
            None => {
                slots.push(0);
                slots.len() - 1
            }
        };
        slots[slot] = last;
        through.insert(e, (g, slot));
    }
    for (g, slots) in ends {
        if let Some(cap) = caps.get_mut(g) {
            cap.raise(Zone::Pass, slots.len());
        }
    }
    through
}
