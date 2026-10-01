//! Left-to-right order inside a gutter, so wires drop nearly vertically.
//!
//! Each node is keyed by the median column of the pills it wires (sources:
//! their triggers; events: their emitters, or the pills their handlers fire
//! into when nothing emits them; controllers: their fires). Nodes with no
//! pill at all go last. Ties go sources, events, controllers, then
//! definition order.
//!
//! A controller that handles an event in the same gutter goes right after
//! the last such event instead, so the subscription is a short hop to the
//! right (the engine needs it to point right anyway: both ends have fixed
//! columns).

use std::cmp::Ordering;
use std::collections::BTreeMap;

use super::{Assignment, Gutter, LaneEnd, Wires};

/// A wiring node, by its index among the model's sources, events or
/// controllers. The variant order is the tie-break order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum WiringNode {
    Source(usize),
    Event(usize),
    Controller(usize),
}

/// The median of the pills' columns, `None` without pills.
pub(crate) fn median(ends: &[LaneEnd]) -> Option<f32> {
    let mut xs: Vec<f32> = ends.iter().map(|e| e.column).collect();
    if xs.is_empty() {
        return None;
    }
    xs.sort_by(f32::total_cmp);
    let mid = xs.len() / 2;
    Some(if xs.len() % 2 == 1 { xs[mid] } else { (xs[mid - 1] + xs[mid]) / 2.0 })
}

/// A node's sort key (see the module docs).
fn key(wires: &Wires, node: WiringNode) -> Option<f32> {
    match node {
        WiringNode::Source(s) => wires.sources.get(s).and_then(|s| median(&s.triggers)),
        WiringNode::Event(e) => wires.events.get(e).and_then(|e| {
            median(&e.emitters).or_else(|| median(&e.handled_into.iter().flatten().copied().collect::<Vec<_>>()))
        }),
        WiringNode::Controller(c) => wires.controllers.get(c).and_then(|c| median(&c.fires)),
    }
}

fn by_key(a: (Option<f32>, WiringNode), b: (Option<f32>, WiringNode)) -> Ordering {
    let keyed = match (a.0, b.0) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    };
    keyed.then(a.1.cmp(&b.1))
}

/// Every gutter's nodes, left to right.
pub(crate) fn order(wires: &Wires, assignment: &Assignment) -> BTreeMap<Gutter, Vec<WiringNode>> {
    let mut members: BTreeMap<Gutter, Vec<WiringNode>> = BTreeMap::new();
    let all = assignment
        .sources
        .iter()
        .enumerate()
        .map(|(i, &g)| (g, WiringNode::Source(i)))
        .chain(assignment.events.iter().enumerate().map(|(i, &g)| (g, WiringNode::Event(i))))
        .chain(assignment.controllers.iter().enumerate().map(|(i, &g)| (g, WiringNode::Controller(i))));
    for (g, node) in all {
        members.entry(g).or_default().push(node);
    }

    let mut out = BTreeMap::new();
    for (gutter, nodes) in members {
        // Controllers following one of their events in this gutter.
        let attached = |c: usize| -> Vec<usize> {
            wires.controllers.get(c).map_or_else(Vec::new, |w| {
                w.events.iter().copied().filter(|&e| assignment.events.get(e) == Some(&gutter)).collect()
            })
        };
        let mut free: Vec<(Option<f32>, WiringNode)> = nodes
            .iter()
            .filter(|n| !matches!(n, WiringNode::Controller(c) if !attached(*c).is_empty()))
            .map(|&n| (key(wires, n), n))
            .collect();
        free.sort_by(|&a, &b| by_key(a, b));
        let position: BTreeMap<WiringNode, usize> = free.iter().enumerate().map(|(i, &(_, n))| (n, i)).collect();
        // Each attached controller hangs off its rightmost event.
        let mut hanging: BTreeMap<usize, Vec<(Option<f32>, WiringNode)>> = BTreeMap::new();
        for &n in &nodes {
            let WiringNode::Controller(c) = n else { continue };
            let last = attached(c).into_iter().filter_map(|e| position.get(&WiringNode::Event(e)).copied()).max();
            if let Some(at) = last {
                hanging.entry(at).or_default().push((key(wires, n), n));
            }
        }
        let mut row = Vec::with_capacity(nodes.len());
        for (i, &(_, n)) in free.iter().enumerate() {
            row.push(n);
            if let Some(mut after) = hanging.remove(&i) {
                after.sort_by(|&a, &b| by_key(a, b));
                row.extend(after.into_iter().map(|(_, n)| n));
            }
        }
        out.insert(gutter, row);
    }
    out
}
