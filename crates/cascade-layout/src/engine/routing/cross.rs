//! Edges between bands (groups). They never cross a foreign band's
//! interior: an edge leaves its band through the band's top or bottom
//! boundary (from a track in the channel beside its node), runs
//! horizontally in the gap next to its band, and
//!
//! - if the target band borders that gap, drops straight into it;
//! - otherwise runs along a corridor left or right of every band to the gap
//!   bordering the target band, then across and in.
//!
//! Horizontal runs in each gap and vertical runs in each corridor get
//! tracks like channel segments do.

use std::collections::BTreeMap;

use super::super::tracks::{Toward, TrackSeg, assign};

/// The route skeleton of one cross-band edge.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CrossPlan {
    pub edge: usize,
    /// The target band is below the source band.
    pub down: bool,
    /// Gap entered on leaving the source band.
    pub first_gap: usize,
    /// Gap bordering the target band (equal to `first_gap` when adjacent).
    pub last_gap: usize,
    /// `Some(true)` for the right corridor, `Some(false)` for the left.
    pub corridor: Option<bool>,
    /// Channel positions of the legs in the source and target bands.
    pub x_s: f32,
    pub x_t: f32,
}

/// Where the bands sit: stacking order and the corridors beside them.
#[derive(Clone, Debug)]
pub(crate) struct Stack {
    /// Stack position of each band, if it is stacked.
    pub position: Vec<Option<usize>>,
    /// Left edge of the left corridor's first track and of the right one.
    pub left_base: f32,
    pub right_base: f32,
    pub edge_spacing: f32,
}

pub(crate) fn plan(edge: usize, s_band: usize, t_band: usize, x_s: f32, x_t: f32, stack: &Stack) -> Option<CrossPlan> {
    let (s, t) = (stack.position[s_band]?, stack.position[t_band]?);
    let down = s < t;
    let (first_gap, last_gap) = if down { (s, t - 1) } else { (s - 1, t) };
    let corridor = (first_gap != last_gap).then(|| {
        let right = (x_s - stack.right_base).abs() + (stack.right_base - x_t).abs();
        let left = (x_s - stack.left_base).abs() + (stack.left_base - x_t).abs();
        left >= right
    });
    Some(CrossPlan { edge, down, first_gap, last_gap, corridor, x_s, x_t })
}

fn corridor_base(stack: &Stack, right: bool) -> f32 {
    if right { stack.right_base } else { stack.left_base }
}

/// Horizontal runs: per plan, the first-gap run and (with a corridor) the
/// last-gap run, as (gap, plan index, is_last, segment).
fn gap_runs(
    plans: &[CrossPlan],
    stack: &Stack,
    nets: &dyn Fn(usize) -> [Option<u64>; 2],
) -> Vec<(usize, usize, bool, TrackSeg)> {
    let mut out = Vec::new();
    let arrive = |down: bool| if down { Toward::Low } else { Toward::High };
    let leave = |down: bool| if down { Toward::High } else { Toward::Low };
    for (i, p) in plans.iter().enumerate() {
        let [src_net, dst_net] = nets(p.edge);
        match p.corridor {
            None => out.push((
                p.first_gap,
                i,
                false,
                TrackSeg {
                    lo: p.x_s.min(p.x_t),
                    hi: p.x_s.max(p.x_t),
                    joins: vec![(p.x_s, arrive(p.down)), (p.x_t, leave(p.down))],
                    nets: [src_net, dst_net],
                },
            )),
            Some(right) => {
                let xc = corridor_base(stack, right);
                out.push((
                    p.first_gap,
                    i,
                    false,
                    TrackSeg {
                        lo: p.x_s.min(xc),
                        hi: p.x_s.max(xc),
                        joins: vec![(p.x_s, arrive(p.down)), (xc, leave(p.down))],
                        nets: [src_net, None],
                    },
                ));
                out.push((
                    p.last_gap,
                    i,
                    true,
                    TrackSeg {
                        lo: p.x_t.min(xc),
                        hi: p.x_t.max(xc),
                        joins: vec![(xc, arrive(p.down)), (p.x_t, leave(p.down))],
                        nets: [None, dst_net],
                    },
                ));
            }
        }
    }
    out
}

/// Number of tracks each gap needs (for sizing gaps before stacking).
pub(crate) fn gap_track_counts(
    plans: &[CrossPlan],
    stack: &Stack,
    gaps: usize,
    nets: &dyn Fn(usize) -> [Option<u64>; 2],
) -> Vec<usize> {
    let runs = gap_runs(plans, stack, nets);
    let mut counts = vec![0usize; gaps];
    let mut by_gap: BTreeMap<usize, Vec<TrackSeg>> = BTreeMap::new();
    for (g, _, _, seg) in runs {
        by_gap.entry(g).or_default().push(seg);
    }
    for (g, segs) in by_gap {
        if let Some(c) = counts.get_mut(g) {
            *c = assign(&segs).1;
        }
    }
    counts
}

/// Resolved coordinates of one cross-band route.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CrossGeometry {
    pub x_s: f32,
    /// Height of the run in the first gap.
    pub y_first: f32,
    /// Corridor track position, when the route uses one.
    pub x_corridor: Option<f32>,
    /// Height of the run in the last gap.
    pub y_last: f32,
    pub x_t: f32,
}

/// Assign gap and corridor tracks and resolve every plan's coordinates.
/// `gaps[g]` is gap `g`'s (top, bottom).
pub(crate) fn resolve(
    plans: &[CrossPlan],
    stack: &Stack,
    gaps: &[(f32, f32)],
    nets: &dyn Fn(usize) -> [Option<u64>; 2],
) -> Vec<CrossGeometry> {
    let runs = gap_runs(plans, stack, nets);
    let mut by_gap: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (k, run) in runs.iter().enumerate() {
        by_gap.entry(run.0).or_default().push(k);
    }
    let mut run_y = vec![0.0f32; runs.len()];
    for (g, members) in by_gap {
        let segs: Vec<TrackSeg> = members.iter().map(|&k| runs[k].3.clone()).collect();
        let (tracks, n) = assign(&segs);
        let (top, bottom) = gaps.get(g).copied().unwrap_or((0.0, 0.0));
        for (m, &k) in members.iter().enumerate() {
            run_y[k] = top + (tracks[m] as f32 + 1.0) * (bottom - top) / (n.max(1) as f32 + 1.0);
        }
    }
    let mut first_y = vec![0.0f32; plans.len()];
    let mut last_y = vec![None; plans.len()];
    for (k, run) in runs.iter().enumerate() {
        if run.2 {
            last_y[run.1] = Some(run_y[k]);
        } else {
            first_y[run.1] = run_y[k];
        }
    }

    // Corridor tracks.
    let mut corridor_x = vec![None; plans.len()];
    for right in [true, false] {
        let members: Vec<usize> = (0..plans.len()).filter(|&i| plans[i].corridor == Some(right)).collect();
        if members.is_empty() {
            continue;
        }
        let join = if right { Toward::Low } else { Toward::High };
        let segs: Vec<TrackSeg> = members
            .iter()
            .map(|&i| {
                let (a, b) = (first_y[i], last_y[i].unwrap_or(first_y[i]));
                TrackSeg { lo: a.min(b), hi: a.max(b), joins: vec![(a, join), (b, join)], nets: [None, None] }
            })
            .collect();
        let (tracks, _) = assign(&segs);
        let base = corridor_base(stack, right);
        for (m, &i) in members.iter().enumerate() {
            let offset = tracks[m] as f32 * stack.edge_spacing;
            corridor_x[i] = Some(if right { base + offset } else { base - offset });
        }
    }

    plans
        .iter()
        .enumerate()
        .map(|(i, p)| CrossGeometry {
            x_s: p.x_s,
            y_first: first_y[i],
            x_corridor: corridor_x[i],
            y_last: last_y[i].unwrap_or(first_y[i]),
            x_t: p.x_t,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack() -> Stack {
        Stack { position: vec![Some(0), Some(1), Some(2)], left_base: -20.0, right_base: 520.0, edge_spacing: 8.0 }
    }

    #[test]
    fn adjacent_bands_use_one_gap_and_no_corridor() {
        let p = plan(0, 0, 1, 100.0, 200.0, &stack()).expect("plan");
        assert!(p.down && p.first_gap == 0 && p.last_gap == 0 && p.corridor.is_none());
        let up = plan(0, 2, 1, 100.0, 200.0, &stack()).expect("plan");
        assert!(!up.down && up.first_gap == 1 && up.last_gap == 1);
    }

    #[test]
    fn distant_bands_take_the_nearer_corridor() {
        let p = plan(0, 0, 2, 480.0, 500.0, &stack()).expect("plan");
        assert_eq!((p.first_gap, p.last_gap, p.corridor), (0, 1, Some(true)));
        let q = plan(0, 2, 0, 10.0, 0.0, &stack()).expect("plan");
        assert_eq!((q.first_gap, q.last_gap, q.corridor), (1, 0, Some(false)));
    }

    #[test]
    fn resolved_routes_sit_inside_their_gaps() {
        let s = stack();
        let plans = vec![
            plan(0, 0, 2, 100.0, 300.0, &s).expect("p"),
            plan(1, 0, 1, 150.0, 250.0, &s).expect("p"),
            plan(2, 2, 0, 120.0, 90.0, &s).expect("p"),
        ];
        let geo = resolve(&plans, &s, &[(100.0, 140.0), (240.0, 280.0)], &|_| [None, None]);
        for g in &geo {
            assert!(g.y_first > 100.0 && g.y_first < 280.0);
        }
        assert!(geo[0].x_corridor.is_some() && geo[1].x_corridor.is_none());
        assert!((geo[0].y_first - geo[1].y_first).abs() > 1.0, "shared gap runs are separated");
        assert!(geo[0].x_corridor != geo[2].x_corridor);
    }
}
