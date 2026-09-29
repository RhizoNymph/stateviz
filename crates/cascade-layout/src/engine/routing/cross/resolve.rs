//! Turning cross-band plans into coordinates.
//!
//! 1. Verticals sharing a passage are ordered by where they come from and
//!    go to, and placed as close to their planned line as the edge spacing
//!    between them allows.
//! 2. Every gap's horizontal runs get tracks ([`assign`]: for each pair the
//!    order causing fewer crossings, judged from where their verticals
//!    join), spread evenly across the gap.
//! 3. Every corridor's verticals get tracks the same way, which nests them
//!    by span, growing outward from the bands.

use std::collections::BTreeMap;

use super::super::super::packing::{Target, place_l1};
use super::super::super::tracks::{Toward, TrackSeg, assign};
use super::{Corridor, CrossPlan, Passage, Stack, Via};

/// Resolved coordinates of one cross-band route.
#[derive(Clone, Debug)]
pub(crate) struct CrossGeometry {
    /// Main-axis position of every vertical: the source leg, each via, the
    /// target leg.
    pub xs: Vec<f32>,
    /// Height of the run in each gap the route crosses (travel order);
    /// `None` where it stays in one corridor through the gap.
    pub runs: Vec<Option<f32>>,
}

/// Which way the verticals at a run's ends head.
fn arrive(down: bool) -> Toward {
    if down { Toward::Low } else { Toward::High }
}

fn leave(down: bool) -> Toward {
    if down { Toward::High } else { Toward::Low }
}

/// Every run as (gap, plan index, run index, segment), given station
/// positions.
fn gap_runs(
    plans: &[CrossPlan],
    xs: &[Vec<f32>],
    nets: &dyn Fn(usize) -> [Option<u64>; 2],
) -> Vec<(usize, usize, usize, TrackSeg)> {
    let mut out = Vec::new();
    for (i, p) in plans.iter().enumerate() {
        let [src_net, dst_net] = nets(p.edge);
        let last = p.gaps.len().saturating_sub(1);
        for (k, &gap) in p.gaps.iter().enumerate() {
            if !p.has_run(k) {
                continue;
            }
            let (a, b) = (xs[i][k], xs[i][k + 1]);
            out.push((
                gap,
                i,
                k,
                TrackSeg {
                    lo: a.min(b),
                    hi: a.max(b),
                    joins: vec![(a, arrive(p.down)), (b, leave(p.down))],
                    nets: [(k == 0).then_some(src_net).flatten(), (k == last).then_some(dst_net).flatten()],
                },
            ));
        }
    }
    out
}

/// Place the verticals sharing each passage: ordered by the midpoint of
/// where they come from and go to (so fewer runs cross), then as near their
/// planned lines as the edge spacing allows (exactly, in L1, each weighing
/// one more for every neighbouring vertical it lines up with), inside the
/// passage.
fn passage_positions(plans: &[CrossPlan], passages: &[Vec<Passage>], es: f32) -> Vec<Vec<f32>> {
    const STRAIGHT: f32 = 0.5;
    let planned: Vec<Vec<f32>> = plans.iter().map(CrossPlan::stations).collect();
    let mut xs = planned.clone();
    let mut members: BTreeMap<(usize, usize), Vec<(usize, usize)>> = BTreeMap::new();
    for (i, p) in plans.iter().enumerate() {
        for (k, via) in p.vias.iter().enumerate() {
            if let Via::Passage { band, passage } = *via {
                members.entry((band, passage)).or_default().push((i, k + 1));
            }
        }
    }
    for ((band, passage), mut list) in members {
        let Some(room) = passages.get(band).and_then(|b| b.get(passage)) else { continue };
        let middle = |&(i, k): &(usize, usize)| (planned[i][k - 1] + planned[i][k + 1]) / 2.0;
        list.sort_by(|a, b| {
            middle(a)
                .total_cmp(&middle(b))
                .then(planned[a.0][a.1].total_cmp(&planned[b.0][b.1]))
                .then(plans[a.0].edge.cmp(&plans[b.0].edge))
        });
        let gaps: Vec<f32> = (0..list.len()).map(|j| if j == 0 { 0.0 } else { es }).collect();
        let targets: Vec<Vec<Target>> = list
            .iter()
            .map(|&(i, k)| {
                let x = planned[i][k];
                let lined_up = [k - 1, k + 1].iter().filter(|&&j| (planned[i][j] - x).abs() < STRAIGHT).count();
                vec![Target { value: x, weight: 1.0 + lined_up as f32 }]
            })
            .collect();
        let current: Vec<f32> = list.iter().map(|&(i, k)| planned[i][k]).collect();
        let mut pos = place_l1(&gaps, &targets, &current);
        let n = pos.len();
        for j in 0..n {
            let floor = if j == 0 { room.lo } else { pos[j - 1] + es };
            pos[j] = pos[j].max(floor);
        }
        for j in (0..n).rev() {
            let ceiling = if j + 1 == n { room.hi } else { pos[j + 1] - es };
            pos[j] = pos[j].min(ceiling);
        }
        for (&(i, k), x) in list.iter().zip(pos) {
            xs[i][k] = x;
        }
    }
    xs
}

/// Number of tracks each of `gaps` gaps needs (for sizing gaps before
/// stacking).
pub(crate) fn gap_track_counts(
    plans: &[CrossPlan],
    passages: &[Vec<Passage>],
    stack: &Stack,
    gaps: usize,
    nets: &dyn Fn(usize) -> [Option<u64>; 2],
) -> Vec<usize> {
    let xs = passage_positions(plans, passages, stack.edge_spacing);
    let mut by_gap: BTreeMap<usize, Vec<TrackSeg>> = BTreeMap::new();
    for (g, _, _, seg) in gap_runs(plans, &xs, nets) {
        by_gap.entry(g).or_default().push(seg);
    }
    let mut counts = vec![0usize; gaps];
    for (g, segs) in by_gap {
        if let Some(c) = counts.get_mut(g) {
            *c = assign(&segs).1;
        }
    }
    counts
}

/// Assign passage, gap and corridor tracks and resolve every plan's
/// coordinates. `gaps[g]` is gap `g`'s (top, bottom).
pub(crate) fn resolve(
    plans: &[CrossPlan],
    passages: &[Vec<Passage>],
    stack: &Stack,
    gaps: &[(f32, f32)],
    nets: &dyn Fn(usize) -> [Option<u64>; 2],
) -> Vec<CrossGeometry> {
    let mut xs = passage_positions(plans, passages, stack.edge_spacing);

    let runs = gap_runs(plans, &xs, nets);
    let mut by_gap: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (r, run) in runs.iter().enumerate() {
        by_gap.entry(run.0).or_default().push(r);
    }
    let mut run_y: Vec<Vec<Option<f32>>> = plans.iter().map(|p| vec![None; p.gaps.len()]).collect();
    for (g, members) in by_gap {
        let segs: Vec<TrackSeg> = members.iter().map(|&r| runs[r].3.clone()).collect();
        let (tracks, n) = assign(&segs);
        let (top, bottom) = gaps.get(g).copied().unwrap_or((0.0, 0.0));
        for (m, &r) in members.iter().enumerate() {
            let y = top + (tracks[m] as f32 + 1.0) * (bottom - top) / (n.max(1) as f32 + 1.0);
            run_y[runs[r].1][runs[r].2] = Some(y);
        }
    }

    // Corridor verticals: each stretch of consecutive vias in one corridor
    // runs from the run before it to the run after it.
    for side in [Corridor::Right, Corridor::Left] {
        let mut stretches: Vec<(usize, usize, usize)> = Vec::new();
        for (i, p) in plans.iter().enumerate() {
            let mut k = 1;
            while k + 1 < xs[i].len() {
                if p.corridor_at(k) != Some(side) {
                    k += 1;
                    continue;
                }
                let first = k;
                while k + 2 < xs[i].len() && p.corridor_at(k + 1) == Some(side) {
                    k += 1;
                }
                stretches.push((i, first, k));
                k += 1;
            }
        }
        if stretches.is_empty() {
            continue;
        }
        let join = if side == Corridor::Right { Toward::Low } else { Toward::High };
        let segs: Vec<TrackSeg> = stretches
            .iter()
            .map(|&(i, first, last)| {
                let a = run_y[i][first - 1].unwrap_or(0.0);
                let b = run_y[i][last].unwrap_or(a);
                TrackSeg { lo: a.min(b), hi: a.max(b), joins: vec![(a, join), (b, join)], nets: [None, None] }
            })
            .collect();
        let (tracks, _) = assign(&segs);
        let base = stack.base(side);
        for (&(i, first, last), t) in stretches.iter().zip(tracks) {
            let offset = t as f32 * stack.edge_spacing;
            let x = if side == Corridor::Right { base + offset } else { base - offset };
            for station in &mut xs[i][first..=last] {
                *station = x;
            }
        }
    }

    xs.into_iter().zip(run_y).map(|(xs, runs)| CrossGeometry { xs, runs }).collect()
}

#[cfg(test)]
mod tests {
    use super::super::planner::{Request, plan_all};
    use super::*;

    fn stack() -> Stack {
        Stack {
            position: vec![Some(0), Some(1), Some(2)],
            order: vec![0, 1, 2],
            span: (0.0, 500.0),
            left_base: -20.0,
            right_base: 520.0,
            edge_spacing: 8.0,
        }
    }

    fn request(edge: usize, s: usize, t: usize, x_s: f32, x_t: f32) -> Request {
        Request { edge, source_band: s, target_band: t, x_s, x_t }
    }

    const GAPS: [(f32, f32); 2] = [(100.0, 140.0), (240.0, 280.0)];

    #[test]
    fn resolved_routes_sit_inside_their_gaps() {
        let s = stack();
        let mut passages = vec![Vec::new(); 3];
        let requests = [request(0, 0, 2, 100.0, 300.0), request(1, 0, 1, 150.0, 250.0), request(2, 2, 0, 120.0, 90.0)];
        let plans = plan_all(&requests, &s, &mut passages);
        let geo = resolve(&plans, &passages, &s, &GAPS, &|_| [None, None]);
        for (p, g) in plans.iter().zip(&geo) {
            for (k, y) in g.runs.iter().enumerate() {
                if let Some(y) = y {
                    let (top, bottom) = GAPS[p.gaps[k]];
                    assert!(*y > top && *y < bottom, "{geo:?}");
                }
            }
        }
        // Without passages the far routes take corridors, on the side they
        // are nearer to, each on its own track.
        assert!(geo[0].xs[1] < 0.0 && geo[2].xs[1] < 0.0 && geo[0].xs[1] != geo[2].xs[1], "{geo:?}");
        assert!((geo[0].runs[0].unwrap_or(0.0) - geo[1].runs[0].unwrap_or(0.0)).abs() > 1.0, "shared gap runs");
    }

    #[test]
    fn verticals_sharing_a_passage_keep_apart_in_order() {
        let s = stack();
        let mut passages =
            vec![Vec::new(), vec![Passage { lo: 200.0, hi: 300.0, crossings: 0, capacity: 13 }], Vec::new()];
        let requests = [request(0, 0, 2, 250.0, 250.0), request(1, 0, 2, 251.0, 100.0), request(2, 0, 2, 400.0, 400.0)];
        let plans = plan_all(&requests, &s, &mut passages);
        let geo = resolve(&plans, &passages, &s, &GAPS, &|_| [None, None]);
        let x: Vec<f32> = geo.iter().map(|g| g.xs[1]).collect();
        // The route heading left sits left of the one heading on down, and
        // the straight one keeps its line.
        assert_eq!(x[0], 250.0, "{x:?}");
        assert!(x[0] - x[1] >= 8.0 - 1e-3, "{x:?}");
        assert_eq!(x[2], 300.0);
    }
}
