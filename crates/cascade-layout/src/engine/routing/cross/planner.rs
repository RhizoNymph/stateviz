//! Choosing how each cross-band route passes the bands between its ends.
//!
//! For every band in between, the options are the band's passages with
//! room left and the two corridors. A shortest-path search over the bands
//! (a small dynamic program) picks one option per band, minimising
//!
//! - horizontal travel between consecutive verticals,
//! - a fixed cost per jog, so straight routes win ties,
//! - a cost per in-band link a passage crosses,
//! - and a prohibitive cost per band passed in a corridor, so corridors are
//!   used only where a band has no passage with room.
//!
//! A vertical in a passage lines up with the previous vertical wherever the
//! passage allows. Routes are planned shortest first (fewest bands in
//! between, then by edge), each taking one place in every passage it uses.

use super::{Corridor, CrossPlan, Passage, Stack, Via};

/// Cost of a jog (a horizontal run between two verticals that do not line
/// up), in length units.
const JOG: f32 = 24.0;
/// Cost of crossing one link inside a band.
const CROSSING: f32 = 300.0;
/// Cost of passing one band in a corridor.
const CORRIDOR: f32 = 1.0e6;
/// Verticals closer than this line up.
const ALIGNED: f32 = 0.5;

/// One cross-band edge to plan.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Request {
    pub edge: usize,
    pub source_band: usize,
    pub target_band: usize,
    /// Positions of the legs in the source and target bands.
    pub x_s: f32,
    pub x_t: f32,
}

#[derive(Clone, Copy, Debug)]
struct State {
    cost: f32,
    x: f32,
    /// Option index in the previous band (`usize::MAX` for the source leg).
    from: usize,
}

fn step(from: f32, to: f32) -> f32 {
    let d = (to - from).abs();
    if d > ALIGNED { d + JOG } else { d }
}

/// Plan one route through the bands `between` (in travel order), taking a
/// place in each passage it uses.
fn plan_one(
    request: &Request,
    between: &[usize],
    stack: &Stack,
    passages: &mut [Vec<Passage>],
) -> (Vec<Via>, Vec<f32>) {
    let corridors = [Corridor::Left, Corridor::Right];
    let options = |band: usize, passages: &[Vec<Passage>]| -> Vec<Via> {
        let mut out: Vec<Via> = passages[band]
            .iter()
            .enumerate()
            .filter(|(_, p)| p.capacity > 0)
            .map(|(passage, _)| Via::Passage { band, passage })
            .collect();
        out.extend(corridors.iter().map(|&side| Via::Corridor(side)));
        out
    };
    let place = |via: Via, x: f32, passages: &[Vec<Passage>]| -> (f32, f32) {
        match via {
            Via::Passage { band, passage } => {
                let p = &passages[band][passage];
                (p.clamp(x), p.crossings as f32 * CROSSING)
            }
            Via::Corridor(side) => (stack.base(side), CORRIDOR),
            Via::Reserved { .. } => (x, 0.0),
        }
    };

    let mut layers: Vec<(Vec<Via>, Vec<State>)> = Vec::with_capacity(between.len());
    for (k, &band) in between.iter().enumerate() {
        let opts = options(band, passages);
        let prev: Vec<State> = match k.checked_sub(1) {
            Some(j) => layers[j].1.clone(),
            None => vec![State { cost: 0.0, x: request.x_s, from: usize::MAX }],
        };
        let states: Vec<State> = opts
            .iter()
            .map(|&via| {
                let mut best: Option<State> = None;
                for (i, s) in prev.iter().enumerate() {
                    let (x, extra) = place(via, s.x, passages);
                    let cost = s.cost + step(s.x, x) + extra;
                    if best.is_none_or(|b| cost < b.cost) {
                        best = Some(State { cost, x, from: if k == 0 { usize::MAX } else { i } });
                    }
                }
                best.unwrap_or(State { cost: f32::INFINITY, x: request.x_s, from: usize::MAX })
            })
            .collect();
        layers.push((opts, states));
    }

    let Some((_, last)) = layers.last() else { return (Vec::new(), Vec::new()) };
    let mut pick = 0usize;
    let mut best = f32::INFINITY;
    for (i, s) in last.iter().enumerate() {
        let cost = s.cost + step(s.x, request.x_t);
        if cost < best {
            best = cost;
            pick = i;
        }
    }
    let mut vias = vec![Via::Corridor(Corridor::Left); layers.len()];
    let mut xs = vec![0.0f32; layers.len()];
    for k in (0..layers.len()).rev() {
        let (opts, states) = &layers[k];
        vias[k] = opts[pick];
        xs[k] = states[pick].x;
        pick = states[pick].from;
    }
    for via in &vias {
        if let Via::Passage { band, passage } = *via
            && let Some(p) = passages[band].get_mut(passage)
        {
            p.capacity = p.capacity.saturating_sub(1);
        }
    }
    (vias, xs)
}

/// Which way a request travels, the gaps it runs through and the bands in
/// between, in travel order. `None` when an end's band is not stacked.
pub(crate) fn crossing(request: &Request, stack: &Stack) -> Option<(bool, Vec<usize>, Vec<usize>)> {
    let (s, t) = (stack.position[request.source_band]?, stack.position[request.target_band]?);
    let down = s < t;
    let gaps: Vec<usize> = if down { (s..t).collect() } else { (t..s).rev().collect() };
    let between: Vec<usize> = if down { (s + 1..t).collect() } else { (t + 1..s).rev().collect::<Vec<_>>() }
        .into_iter()
        .map(|pos| stack.order[pos])
        .collect();
    Some((down, gaps, between))
}

/// Plan every request. `passages[b]` are band `b`'s passages; their
/// capacities are used up as routes take them.
pub(crate) fn plan_all(requests: &[Request], stack: &Stack, passages: &mut [Vec<Passage>]) -> Vec<CrossPlan> {
    let mut order: Vec<(usize, usize, usize)> = requests
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            let (s, t) = (stack.position[r.source_band]?, stack.position[r.target_band]?);
            Some((s.abs_diff(t), r.edge, i))
        })
        .collect();
    order.sort_unstable();
    let mut plans: Vec<(usize, CrossPlan)> = Vec::with_capacity(order.len());
    for &(_, _, i) in &order {
        let r = &requests[i];
        let Some((down, gaps, between)) = crossing(r, stack) else { continue };
        let (vias, via_x) = plan_one(r, &between, stack, passages);
        plans.push((i, CrossPlan { edge: r.edge, down, gaps, vias, via_x, x_s: r.x_s, x_t: r.x_t }));
    }
    plans.sort_by_key(|(i, _)| *i);
    plans.into_iter().map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(bands: usize) -> Stack {
        Stack {
            position: (0..bands).map(Some).collect(),
            order: (0..bands).collect(),
            span: (0.0, 500.0),
            left_base: -20.0,
            right_base: 520.0,
            edge_spacing: 8.0,
        }
    }

    fn passage(lo: f32, hi: f32, crossings: usize) -> Passage {
        Passage { lo, hi, crossings, capacity: ((hi - lo) / 8.0) as usize + 1 }
    }

    fn request(edge: usize, s: usize, t: usize, x_s: f32, x_t: f32) -> Request {
        Request { edge, source_band: s, target_band: t, x_s, x_t }
    }

    #[test]
    fn adjacent_bands_have_no_vias() {
        let mut passages = vec![Vec::new(); 3];
        let plans = plan_all(&[request(0, 0, 1, 100.0, 200.0), request(1, 2, 1, 50.0, 60.0)], &stack(3), &mut passages);
        assert!(plans[0].down && plans[0].gaps == vec![0] && plans[0].vias.is_empty());
        assert!(!plans[1].down && plans[1].gaps == vec![1] && plans[1].vias.is_empty());
    }

    #[test]
    fn a_band_in_between_is_passed_straight_through_a_passage() {
        let mut passages = vec![Vec::new(), vec![passage(40.0, 60.0, 1), passage(90.0, 300.0, 1)], Vec::new()];
        let plans = plan_all(&[request(0, 0, 2, 120.0, 120.0)], &stack(3), &mut passages);
        assert_eq!(plans[0].vias, vec![Via::Passage { band: 1, passage: 1 }]);
        assert_eq!(plans[0].via_x, vec![120.0]);
        assert_eq!(plans[0].gaps, vec![0, 1]);
        assert_eq!(passages[1][1].capacity, passage(90.0, 300.0, 1).capacity - 1);
    }

    #[test]
    fn fewer_crossings_win_over_a_short_detour() {
        let mut passages = vec![Vec::new(), vec![passage(100.0, 120.0, 4), passage(160.0, 170.0, 0)], Vec::new()];
        let plans = plan_all(&[request(0, 0, 2, 110.0, 110.0)], &stack(3), &mut passages);
        assert_eq!(plans[0].vias, vec![Via::Passage { band: 1, passage: 1 }]);
    }

    #[test]
    fn corridors_only_without_room_on_the_nearer_side() {
        let mut passages = vec![Vec::new(), Vec::new(), Vec::new()];
        let plans = plan_all(&[request(0, 0, 2, 480.0, 450.0), request(1, 2, 0, 10.0, 30.0)], &stack(3), &mut passages);
        assert_eq!(plans[0].vias, vec![Via::Corridor(Corridor::Right)]);
        assert_eq!(plans[1].vias, vec![Via::Corridor(Corridor::Left)]);
        assert_eq!(plans[1].gaps, vec![1, 0]);
    }

    #[test]
    fn full_passages_send_later_routes_elsewhere() {
        let mut passages =
            vec![Vec::new(), vec![Passage { lo: 100.0, hi: 100.0, crossings: 0, capacity: 1 }], Vec::new()];
        let plans =
            plan_all(&[request(0, 0, 2, 100.0, 100.0), request(1, 0, 2, 100.0, 100.0)], &stack(3), &mut passages);
        assert_eq!(plans[0].vias, vec![Via::Passage { band: 1, passage: 0 }]);
        assert!(matches!(plans[1].vias[0], Via::Corridor(_)));
    }
}
