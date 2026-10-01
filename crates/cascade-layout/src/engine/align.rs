//! Aligning bands with each other (`LayoutOptions::align_across_groups`).
//!
//! After every band has its main-axis positions, each freshly placed band
//! shifts its columns along the main axis toward the other ends of its
//! cross-band edges, so edges between stacked bands run as straight as the
//! bands allow. Per band this is a one-dimensional compaction with targets:
//!
//! - the units are the band's columns, where a column holding no node (only
//!   dummies and labels) is welded to the column before it;
//! - units keep their order and their current distances as minimums (a
//!   fresh placement is already packed as tight as its channels allow), and
//!   the first stays at or right of where the fresh placement put it, so
//!   columns only ever spread to the right;
//! - every cross-band edge end pulls its unit so that its leg (the
//!   attachment point of a direct leg, the channel's middle otherwise) lines
//!   up with the other end's, with weight 1, and a feeble pull keeps each
//!   unit where it is;
//! - [`place_l1`] solves it exactly in L1, so edges come out exactly
//!   straight wherever the constraints allow.
//!
//! Bands are solved in stacking order, down and back up, until nothing
//! moves. Bands kept from the previous layout and pinned nodes never move,
//! but freshly placed bands still align with them.

use super::context::{Columns, Ctx};
use super::packing::{Target, place_l1};
use super::problem::EdgeKind;
use super::routing::channels::leg_offset;

const ROUNDS: usize = 8;
const SETTLED: f32 = 0.5;
const STAY: f32 = 1e-3;
/// Holds the first unit at or right of its fresh position.
const FLOOR: f32 = 1e6;

/// One cross-band edge end as seen from its band: its column and the
/// offset of its leg from that column's left edge.
#[derive(Clone, Copy, Debug)]
struct End {
    band: usize,
    column: usize,
    offset: f32,
}

/// A band's columns grouped into units that move together.
struct Units {
    /// Unit of every column.
    unit_of: Vec<usize>,
    /// Offset of every column's left from its unit's.
    offset: Vec<f32>,
    /// Fresh left of every unit.
    start: Vec<f32>,
}

impl Units {
    /// A column holding no node is welded to the column before it.
    fn of(ctx: &Ctx<'_, '_>, band: usize, cols: &Columns) -> Self {
        let bg = &ctx.bands[band];
        let mut unit_of = Vec::with_capacity(cols.count());
        let mut first: Vec<usize> = Vec::new();
        for (l, layer) in bg.layers.iter().enumerate().take(cols.count()) {
            if l == 0 || layer.iter().any(|&i| bg.items[i].is_node()) {
                first.push(l);
            }
            unit_of.push(first.len() - 1);
        }
        let offset = unit_of.iter().enumerate().map(|(l, &u)| cols.left[l] - cols.left[first[u]]).collect();
        let start = first.iter().map(|&l| cols.left[l]).collect();
        Self { unit_of, offset, start }
    }
}

/// The shift of every column of every band (zero for bands that do not
/// move). `movable[b]` says whether band `b` was placed fresh.
pub(crate) fn shifts(ctx: &Ctx<'_, '_>, cols: &[Columns], stack_order: &[usize], movable: &[bool]) -> Vec<Vec<f32>> {
    let p = ctx.p;
    let end_of = |edge: usize, source: bool| -> Option<End> {
        let node = p.edges[edge].end(source).node;
        let band = p.nodes[node].band;
        let item = ctx.item(node)?;
        let offset = leg_offset(ctx, &cols[band], edge, source)?;
        Some(End { band, column: item.layer, offset })
    };
    let pairs: Vec<(End, End)> = (0..p.edges.len())
        .filter(|&e| p.kinds[e] == EdgeKind::CrossBand)
        .filter_map(|e| Some((end_of(e, true)?, end_of(e, false)?)))
        .collect();

    let units: Vec<Units> = (0..cols.len()).map(|b| Units::of(ctx, b, &cols[b])).collect();
    let mut pos: Vec<Vec<f32>> = units.iter().map(|u| u.start.clone()).collect();
    let x_of = |pos: &[Vec<f32>], end: &End| {
        let u = &units[end.band];
        pos[end.band][u.unit_of[end.column]] + u.offset[end.column] + end.offset
    };

    let mut order: Vec<usize> = stack_order.iter().copied().filter(|&b| movable[b] && !pos[b].is_empty()).collect();
    let back: Vec<usize> = order.iter().rev().copied().collect();
    order.extend(back);
    for _ in 0..ROUNDS {
        let mut moved = 0.0f32;
        for &b in &order {
            let Units { unit_of, start, .. } = &units[b];
            let m = start.len();
            let mut targets: Vec<Vec<Target>> = vec![Vec::new(); m + 1];
            targets[0].push(Target { value: start[0], weight: FLOOR });
            for (u, t) in targets.iter_mut().skip(1).enumerate() {
                t.push(Target { value: pos[b][u], weight: STAY });
            }
            for (a, z) in &pairs {
                for (here, there) in [(a, z), (z, a)] {
                    if here.band == b && there.band != b {
                        let delta = x_of(&pos, there) - x_of(&pos, here);
                        let u = unit_of[here.column];
                        targets[u + 1].push(Target { value: pos[b][u] + delta, weight: 1.0 });
                    }
                }
            }
            let mut gaps = vec![0.0f32; m + 1];
            for u in 1..m {
                gaps[u + 1] = start[u] - start[u - 1];
            }
            let mut current = vec![start[0]];
            current.extend_from_slice(&pos[b]);
            let solved = place_l1(&gaps, &targets, &current);
            for u in 0..m {
                moved = moved.max((solved[u + 1] - pos[b][u]).abs());
                pos[b][u] = solved[u + 1];
            }
        }
        if moved < SETTLED {
            break;
        }
    }

    (0..cols.len())
        .map(|b| {
            let u = &units[b];
            (0..cols[b].count()).map(|l| pos[b][u.unit_of[l]] + u.offset[l] - cols[b].left[l]).collect()
        })
        .collect()
}
