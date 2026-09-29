//! Edges between bands (groups).
//!
//! An edge leaves its band through the band's top or bottom boundary, either
//! straight out of a port facing the other band or from a track in the
//! channel beside its node ([`super::channels::Leg`]). It then alternates
//! horizontal runs in the gaps between bands with vertical runs through the
//! bands in between:
//!
//! - between adjacent bands, one run in the gap between them, which is a
//!   single vertical line when the two legs line up;
//! - across a band in between, a vertical through one of the band's free
//!   passages (a strip clear of its nodes, channel tracks and pins,
//!   [`passages`]), chosen by [`planner`] for the shortest, least crossing
//!   route with the fewest jogs;
//! - only when a band has no passage with room left, a side corridor left
//!   or right of every band, whichever is shorter.
//!
//! Runs in each gap, verticals in each passage and verticals in each
//! corridor get tracks ordered to avoid crossings ([`resolve`]).

pub(crate) mod passages;
pub(crate) mod planner;
pub(crate) mod resolve;

pub(crate) use passages::Passage;
pub(crate) use planner::{Request, plan_all};
pub(crate) use resolve::{CrossGeometry, gap_track_counts, resolve};

/// A side corridor, beside every band.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Corridor {
    Left,
    Right,
}

/// How a cross-band route passes one band between its source and target
/// bands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Via {
    /// Straight through the band, in its free passage `passage`.
    Passage { band: usize, passage: usize },
    /// Around the band, in a side corridor.
    Corridor(Corridor),
    /// Straight through the band on a vertical reserved for this edge
    /// (shared layers: a pass slot kept free in every band).
    Reserved { band: usize },
}

/// The route skeleton of one cross-band edge.
#[derive(Clone, Debug)]
pub(crate) struct CrossPlan {
    pub edge: usize,
    /// The target band is below the source band.
    pub down: bool,
    /// The gaps the route runs through, in travel order: one more than
    /// `vias`.
    pub gaps: Vec<usize>,
    /// How it passes each band in between, in travel order.
    pub vias: Vec<Via>,
    /// Planned position of each via (a corridor's base for corridors).
    pub via_x: Vec<f32>,
    /// Positions of the legs in the source and target bands.
    pub x_s: f32,
    pub x_t: f32,
}

impl CrossPlan {
    /// Planned x of every vertical station: source leg, vias, target leg.
    pub(crate) fn stations(&self) -> Vec<f32> {
        let mut xs = Vec::with_capacity(self.vias.len() + 2);
        xs.push(self.x_s);
        xs.extend_from_slice(&self.via_x);
        xs.push(self.x_t);
        xs
    }

    /// The corridor station `k` (0 = source leg) is in, if any.
    pub(crate) fn corridor_at(&self, station: usize) -> Option<Corridor> {
        match station.checked_sub(1).and_then(|k| self.vias.get(k)) {
            Some(Via::Corridor(side)) => Some(*side),
            _ => None,
        }
    }

    /// Whether the run in gap `i` is needed: not when the route stays in one
    /// corridor through that gap.
    pub(crate) fn has_run(&self, i: usize) -> bool {
        !matches!((self.corridor_at(i), self.corridor_at(i + 1)), (Some(a), Some(b)) if a == b)
    }
}

/// Where the bands sit: stacking order, their shared cross extent and the
/// corridors beside them.
#[derive(Clone, Debug)]
pub(crate) struct Stack {
    /// Stack position of each band, if it is stacked.
    pub position: Vec<Option<usize>>,
    /// Stacked bands, top to bottom.
    pub order: Vec<usize>,
    /// Main-axis extent shared by the stacked bands (the lanes' width).
    pub span: (f32, f32),
    /// Left edge of the left corridor's first track and of the right one.
    pub left_base: f32,
    pub right_base: f32,
    pub edge_spacing: f32,
}

impl Stack {
    pub(crate) fn base(&self, side: Corridor) -> f32 {
        match side {
            Corridor::Left => self.left_base,
            Corridor::Right => self.right_base,
        }
    }
}
