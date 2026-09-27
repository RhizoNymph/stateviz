//! Race candidate (info): one originating event leads, through different
//! controllers, to two fires that may hit the same instance of one machine.
//!
//! For each event E, the rules reached in E's forward cascade are the rules
//! of every handler in E's forward cone of the causal graph. Two of them form
//! a candidate when:
//!
//! - they belong to different controllers (one controller orders its own
//!   rules),
//! - they fire into the same machine, and each fire can land (its trigger is
//!   accepted by some transition; invalid fires are reported as errors
//!   instead),
//! - their targets may alias: a spawned instance (`new …`) never aliases;
//!   a singleton (`Machine` with no predicates) aliases any other target on
//!   that machine; selectors (`where` / `all … where`) are assumed to alias
//!   since predicates are not compared,
//! - neither rule is causally downstream of the other rule's fired
//!   transitions (then the order is causal, not a race).
//!
//! Each pair is reported once, with the origin event closest to both: the
//! fewest transitions between the event and the two rules combined, ties
//! going to the event first in model order.

use std::collections::BTreeMap;

use super::describe;
use super::{Finding, FindingDetail};
use crate::causal::{CausalGraph, CausalNode, Direction};
use crate::ids::{ControllerId, EventId, HandlerId, MachineId, RuleId};
use crate::model::{Model, Target};

/// Which instances a rule's fire can reach, as far as aliasing goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InstanceScope {
    /// A newly spawned instance; nothing else can hold a reference to it yet.
    Fresh,
    /// The one instance of a singleton machine.
    Singleton,
    /// Instances chosen by field predicates, which are not compared.
    Selected,
}

impl InstanceScope {
    fn of(target: &Target) -> Self {
        match target {
            Target::Spawn { .. } => InstanceScope::Fresh,
            Target::One { predicates } if predicates.is_empty() => InstanceScope::Singleton,
            Target::One { .. } | Target::All { .. } => InstanceScope::Selected,
        }
    }

    fn may_alias(self, other: Self) -> bool {
        match (self, other) {
            (InstanceScope::Fresh, _) | (_, InstanceScope::Fresh) => false,
            (
                InstanceScope::Singleton | InstanceScope::Selected,
                InstanceScope::Singleton | InstanceScope::Selected,
            ) => true,
        }
    }
}

/// A rule that can take part in a race, with what the check needs of it.
#[derive(Clone, Copy, Debug)]
struct Racer {
    rule: RuleId,
    controller: ControllerId,
    handler: HandlerId,
    machine: MachineId,
    scope: InstanceScope,
}

/// The origin chosen for a pair so far.
#[derive(Clone, Copy, Debug)]
struct Origin {
    distance: u32,
    event: EventId,
}

pub(super) fn check(model: &Model, graph: &CausalGraph) -> Vec<Finding> {
    // Indexed by rule; `None` for rules that cannot race.
    let racers: Vec<Option<Racer>> = model
        .rules()
        .map(|(rule, r)| {
            let trigger = model.trigger(r.trigger);
            let scope = InstanceScope::of(&r.target);
            (!trigger.accepted_by.is_empty() && scope != InstanceScope::Fresh).then_some(Racer {
                rule,
                controller: r.controller,
                handler: r.handler,
                machine: trigger.machine,
                scope,
            })
        })
        .collect();
    let downstream = Downstream::compute(model, graph, &racers);

    let mut best: BTreeMap<(RuleId, RuleId), Origin> = BTreeMap::new();
    for event in model.event_ids() {
        let Some(seed) = graph.ix_of(CausalNode::Event(event)) else {
            continue;
        };
        let cone = graph.cone(&[seed], Direction::Forward, None);
        // (racer, hops from the event to the racer's handler)
        let mut reached: Vec<(Racer, u32)> = Vec::new();
        for (node, hops) in cone.nodes() {
            if let CausalNode::Handler(h) = graph.node(node) {
                reached
                    .extend(model.handler(h).rules.iter().filter_map(|r| racers[r.index()]).map(|racer| (racer, hops)));
            }
        }
        reached.sort_by_key(|(racer, _)| (racer.machine, racer.rule));

        for group in reached.chunk_by(|a, b| a.0.machine == b.0.machine) {
            for (i, &(a, hops_a)) in group.iter().enumerate() {
                for &(b, hops_b) in &group[i + 1..] {
                    if a.controller == b.controller
                        || !a.scope.may_alias(b.scope)
                        || downstream.reaches(a.rule, b.handler)
                        || downstream.reaches(b.rule, a.handler)
                    {
                        continue;
                    }
                    let candidate = Origin { distance: hops_a + hops_b, event };
                    best.entry((a.rule, b.rule))
                        .and_modify(|current| {
                            // Events are visited in model order, so only a
                            // strictly closer event replaces the current one.
                            if candidate.distance < current.distance {
                                *current = candidate;
                            }
                        })
                        .or_insert(candidate);
                }
            }
        }
    }

    best.into_iter()
        .map(|((first, second), origin)| {
            let machine = model.trigger(model.rule(first).trigger).machine;
            Finding::new(
                FindingDetail::RaceCandidate { origin: origin.event, machine, first, second },
                message(model, origin.event, first, second),
            )
        })
        .collect()
}

/// For each racing rule, the handlers reachable from the transitions it
/// fires.
struct Downstream {
    /// Indexed by rule, then by handler.
    handlers: Vec<Vec<bool>>,
}

impl Downstream {
    fn compute(model: &Model, graph: &CausalGraph, racers: &[Option<Racer>]) -> Self {
        let handlers = racers
            .iter()
            .map(|racer| {
                let Some(racer) = racer else {
                    return Vec::new();
                };
                let seeds: Vec<_> = model
                    .trigger(model.rule(racer.rule).trigger)
                    .accepted_by
                    .iter()
                    .filter_map(|&t| graph.ix_of(CausalNode::Transition(t)))
                    .collect();
                let cone = graph.cone(&seeds, Direction::Forward, None);
                model
                    .handler_ids()
                    .map(|h| graph.ix_of(CausalNode::Handler(h)).is_some_and(|ix| cone.contains(ix)))
                    .collect()
            })
            .collect();
        Self { handlers }
    }

    /// Whether `handler` runs as a consequence of `rule`'s fired transitions.
    fn reaches(&self, rule: RuleId, handler: HandlerId) -> bool {
        self.handlers.get(rule.index()).and_then(|hs| hs.get(handler.index())).copied().unwrap_or(false)
    }
}

/// `PaymentAuthorized leads Billing to fire Payment.capture and FraudCheck to
/// fire Payment.void, which may hit the same Payment instance in either order`.
fn message(model: &Model, origin: EventId, first: RuleId, second: RuleId) -> String {
    let part = |rule: RuleId| {
        let r = model.rule(rule);
        format!("{} to fire {}", model.controller(r.controller).name, describe::trigger(model, r.trigger))
    };
    let machine = &model.machine(model.trigger(model.rule(first).trigger).machine).name;
    format!(
        "{} leads {} and {}, which may hit the same {machine} instance in either order",
        model.event(origin).name,
        part(first),
        part(second)
    )
}
