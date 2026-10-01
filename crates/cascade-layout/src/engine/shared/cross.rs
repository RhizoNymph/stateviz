//! Edges between bands with shared layers.
//!
//! Each edge leaves its band from its leg (an exit-zone track right of its
//! source column, or straight out of a port facing the other band), runs in
//! the gap to its target band's entry leg when the bands are neighbours,
//! and otherwise first to its reserved pass slot, straight through every
//! band in between on that vertical (kept free of tracks in every band),
//! then in the last gap to the entry leg. With the target layer above the
//! source layer, exit, pass and entry positions increase in that order, so
//! both runs head with the flow and no corridor is needed.

use std::collections::BTreeMap;

use super::super::context::Ctx;
use super::super::problem::EdgeKind;
use super::super::routing::channels::SegKey;
use super::super::routing::cross::planner::{Request, crossing};
use super::super::routing::cross::{CrossPlan, Stack, Via};
use super::super::routing::leg_x;
use super::{Shared, Zone};

/// The plan of every edge between stacked bands, in edge order.
pub(crate) fn plans(
    ctx: &Ctx<'_, '_>,
    shared: &Shared,
    seg_x: &[BTreeMap<SegKey, f32>],
    stack: &Stack,
) -> Vec<CrossPlan> {
    let p = ctx.p;
    let mut out = Vec::new();
    for e in 0..p.edges.len() {
        if p.kinds[e] != EdgeKind::CrossBand {
            continue;
        }
        let (Some(x_s), Some(x_t)) = (leg_x(ctx, seg_x, e, true), leg_x(ctx, seg_x, e, false)) else { continue };
        let request = Request {
            edge: e,
            source_band: p.nodes[p.edges[e].source.node].band,
            target_band: p.nodes[p.edges[e].target.node].band,
            x_s,
            x_t,
        };
        let Some((down, gaps, between)) = crossing(&request, stack) else { continue };
        let x = match shared.through.get(&e) {
            Some(&(g, slot)) => {
                let cap = shared.caps.get(g).map_or(1, |c| c.get(Zone::Pass));
                shared.position(g, Zone::Pass, slot, cap)
            }
            None => x_s,
        };
        let vias: Vec<Via> = between.iter().map(|&band| Via::Reserved { band }).collect();
        let via_x = vec![x; vias.len()];
        out.push(CrossPlan { edge: e, down, gaps, vias, via_x, x_s, x_t });
    }
    out
}
