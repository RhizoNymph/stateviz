//! Where the build canvas puts its wiring: in *gutters*, thin layout
//! groups between consecutive machine lanes (plus one above the first lane
//! and one below the last), each wiring node in the gutter next to what it
//! wires.
//!
//! The layout engine stacks groups top to bottom and routes an edge between
//! two groups straight through the gap between them only when they are
//! neighbours; any other edge detours through a side corridor. So a wire is
//! short and direct exactly when its two ends sit in adjacent groups, and
//! the placement here minimises, per wiring node, first the number of its
//! wires that would need a corridor and then how many lane groups they
//! cross:
//!
//! - **Events** by their emits (from the emitting pills) plus, for each
//!   controller handling the event, one wire to the nearest pill its rules
//!   for the event fire into (a stand-in for the subscription, since the
//!   controller will want to sit by those pills). Ties go to the gutter
//!   nearest the handling controllers once those are placed, then below
//!   the emitter.
//! - **Controllers** by their fires plus their subscriptions to the events
//!   already placed. Ties go above the lane they fire into.
//! - **Sources** by their triggers. Ties go above the lane they trigger.
//!
//! Each rule only looks at the node's own wiring (and, for events, the
//! rules handling them), so an edit moves only the wiring nodes whose own
//! wiring changed. Within a gutter, [`order`] sorts nodes by where their
//! pills sit, and [`memo`] keeps every column across edits.

pub(super) mod memo;
pub(super) mod order;

use std::cmp::Ordering;
use std::ops::Add;

/// A gutter, counted from the top: gutter `k` sits right above machine
/// `k`'s lane, and the last one below every lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Gutter(pub usize);

/// The build canvas's layout groups top to bottom (draft group indices are
/// stacking positions): which are gutters, and how many lane groups lie
/// between any two positions.
#[derive(Clone, Debug)]
pub(crate) struct Stack {
    /// Draft group of each gutter, top to bottom.
    gutters: Vec<usize>,
    /// `lanes_before[p]`: lane (non-gutter) groups at positions `< p`.
    lanes_before: Vec<u32>,
}

impl Stack {
    /// `is_gutter[p]` says whether draft group `p` is a gutter.
    pub fn new(is_gutter: &[bool]) -> Self {
        let mut lanes_before = Vec::with_capacity(is_gutter.len() + 1);
        let mut n = 0u32;
        for &g in is_gutter {
            lanes_before.push(n);
            n += u32::from(!g);
        }
        lanes_before.push(n);
        let gutters = is_gutter.iter().enumerate().filter_map(|(p, &g)| g.then_some(p)).collect();
        Self { gutters, lanes_before }
    }

    pub fn gutter_count(&self) -> usize {
        self.gutters.len()
    }

    /// The draft group of a gutter.
    pub fn group(&self, gutter: Gutter) -> Option<usize> {
        self.gutters.get(gutter.0).copied()
    }

    /// Lane groups strictly between two stacking positions.
    pub fn lanes_between(&self, a: usize, b: usize) -> u32 {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        if hi <= lo + 1 {
            return 0;
        }
        let at = |p: usize| self.lanes_before.get(p).or(self.lanes_before.last()).copied().unwrap_or(0);
        at(hi).saturating_sub(at(lo + 1))
    }

    /// The cost of a wire between a gutter and the group at `position`.
    fn wire(&self, gutter: Gutter, position: usize) -> Cost {
        match self.group(gutter) {
            Some(g) => Cost::of_span(self.lanes_between(g, position)),
            None => Cost::of_span(u32::MAX / 4),
        }
    }

    /// The cost of a wire between two gutters.
    fn between(&self, a: Gutter, b: Gutter) -> Cost {
        match (self.group(a), self.group(b)) {
            (Some(x), Some(y)) => Cost::of_span(self.lanes_between(x, y)),
            _ => Cost::of_span(u32::MAX / 4),
        }
    }
}

/// How much a set of wires detours: wires that need a side corridor (they
/// cross at least one lane), then lane groups crossed in total. Compared in
/// that order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Cost {
    pub corridors: u32,
    pub crossed: u32,
}

impl Cost {
    fn of_span(lanes: u32) -> Self {
        Self { corridors: u32::from(lanes > 0), crossed: lanes }
    }
}

impl Add for Cost {
    type Output = Cost;

    fn add(self, rhs: Cost) -> Cost {
        Cost { corridors: self.corridors + rhs.corridors, crossed: self.crossed.saturating_add(rhs.crossed) }
    }
}

impl std::iter::Sum for Cost {
    fn sum<I: Iterator<Item = Cost>>(iter: I) -> Cost {
        iter.fold(Cost::default(), Add::add)
    }
}

/// Where a wire ends in a lane: the stacking position of its group, and a
/// proxy for how far right it sits (the pill's column in its lane).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LaneEnd {
    pub position: usize,
    pub column: f32,
}

/// Which way a tie between equally good gutters breaks, relative to the
/// lanes the node wires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Prefer {
    /// The gutter nearest the top: sources and controllers sit above the
    /// lane they drive, so their wires drop into it.
    Above,
    /// The gutter nearest the bottom: events sit below their emitter, so
    /// the chain reads downwards.
    Below,
}

/// The wiring of one event.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EventWires {
    pub emitters: Vec<LaneEnd>,
    /// Per controller handling this event: the pills its rules for this
    /// event fire into.
    pub handled_into: Vec<Vec<LaneEnd>>,
}

/// The wiring of one controller.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ControllerWires {
    pub fires: Vec<LaneEnd>,
    /// Events it subscribes to, by index.
    pub events: Vec<usize>,
}

/// The wiring of one source.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SourceWires {
    pub triggers: Vec<LaneEnd>,
}

/// Everything the placement looks at, indexed like the model's events,
/// controllers and sources.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Wires {
    pub events: Vec<EventWires>,
    pub controllers: Vec<ControllerWires>,
    pub sources: Vec<SourceWires>,
}

/// The gutter of every wiring node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Assignment {
    pub events: Vec<Gutter>,
    pub controllers: Vec<Gutter>,
    pub sources: Vec<Gutter>,
}

/// The best gutter by `cost`, ties broken by `prefer`. A node with no wires
/// at all (every gutter costs nothing and none is nearer) goes to the last
/// gutter, below everything, where it disturbs nothing.
fn best<C: Ord + Copy>(stack: &Stack, prefer: Prefer, isolated: bool, cost: impl Fn(Gutter) -> C) -> Gutter {
    let last = Gutter(stack.gutter_count().saturating_sub(1));
    if isolated {
        return last;
    }
    let mut best: Option<(C, Gutter)> = None;
    for g in (0..stack.gutter_count()).map(Gutter) {
        let c = cost(g);
        let better = match best {
            None => true,
            Some((bc, bg)) => match c.cmp(&bc) {
                Ordering::Less => true,
                Ordering::Greater => false,
                Ordering::Equal => prefer == Prefer::Below && g > bg,
            },
        };
        if better {
            best = Some((c, g));
        }
    }
    best.map_or(last, |(_, g)| g)
}

/// Place every wiring node (see the module docs).
pub(crate) fn assign(stack: &Stack, wires: &Wires) -> Assignment {
    let lane_cost = |g: Gutter, ends: &[LaneEnd]| ends.iter().map(|e| stack.wire(g, e.position)).sum::<Cost>();
    // One stand-in wire per handling controller, to its nearest pill.
    let handled_cost = |g: Gutter, handlers: &[Vec<LaneEnd>]| {
        handlers.iter().filter_map(|ends| ends.iter().map(|e| stack.wire(g, e.position)).min()).sum::<Cost>()
    };
    let own_cost = |g: Gutter, e: &EventWires| lane_cost(g, &e.emitters) + handled_cost(g, &e.handled_into);
    let wired = |e: &EventWires| !e.emitters.is_empty() || e.handled_into.iter().any(|h| !h.is_empty());
    let first: Vec<Option<Gutter>> =
        wires.events.iter().map(|e| wired(e).then(|| best(stack, Prefer::Below, false, |g| own_cost(g, e)))).collect();
    let controllers: Vec<Gutter> = wires
        .controllers
        .iter()
        .map(|c| {
            let events: Vec<Gutter> = c.events.iter().filter_map(|&e| first.get(e).copied().flatten()).collect();
            let isolated = c.fires.is_empty() && events.is_empty();
            best(stack, Prefer::Above, isolated, |g| {
                lane_cost(g, &c.fires) + events.iter().map(|&e| stack.between(g, e)).sum::<Cost>()
            })
        })
        .collect();
    let handlers_of = |e: usize| -> Vec<Gutter> {
        wires.controllers.iter().zip(&controllers).filter(|(c, _)| c.events.contains(&e)).map(|(_, &g)| g).collect()
    };
    let to_handlers = |g: Gutter, handlers: &[Gutter]| handlers.iter().map(|&h| stack.between(g, h)).sum::<Cost>();
    let events = wires
        .events
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let handlers = handlers_of(i);
            if wired(e) {
                // Among the gutters equally good for the event's own wires,
                // the one nearest its controllers.
                best(stack, Prefer::Below, false, |g| (own_cost(g, e), to_handlers(g, &handlers)))
            } else {
                // Nothing emits it and no rule handles it: with its
                // handling controllers (a handler without rules still
                // subscribes), else last.
                best(stack, Prefer::Below, handlers.is_empty(), |g| to_handlers(g, &handlers))
            }
        })
        .collect();
    let sources = wires
        .sources
        .iter()
        .map(|s| best(stack, Prefer::Above, s.triggers.is_empty(), |g| lane_cost(g, &s.triggers)))
        .collect();
    Assignment { events, controllers, sources }
}

#[cfg(test)]
mod tests;
