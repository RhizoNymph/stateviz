//! Race candidate (info): one originating event leads to two deliveries
//! that may hit the same instance of one machine, in either order.
//!
//! A *party* is something that delivers to an instance:
//!
//! - a rule (a controller's fire), reached when its handler is in the
//!   origin's forward cone; it can race when its trigger is accepted by some
//!   transition (invalid fires are errors instead) and it does not spawn;
//! - a subscription (a machine's subscription trigger on an event), reached
//!   when its event is in the origin's forward cone; it can race when some
//!   transition accepts it.
//!
//! Two parties reached from one event form a candidate when:
//!
//! - they deliver into the same machine;
//! - they are not two rules of one controller (one controller's fires reach
//!   an instance in order);
//! - their targets may alias: a spawned instance (`new …`) never aliases; a
//!   singleton (`Machine` with no predicates) aliases any other target on
//!   that machine; selectors (`where` / `all … where`) and subscriptions
//!   (every live instance whose `where` holds) are assumed to alias, since
//!   instance predicates are not compared;
//! - neither is causally downstream of the other's transitions (then the
//!   order is causal, not a race): a rule is downstream when its handler is
//!   in the forward cone of the other party's accepting transitions, a
//!   subscription when its event is;
//! - two subscriptions are not ordered by emission: each instance receives
//!   events in emission order, so subscriptions to events emitted by exactly
//!   the same transitions, or to an event emitted downstream of the other
//!   event, are ordered (two subscriptions to one event are one party, since
//!   an instance takes at most one transition per delivery);
//! - when both parties react to the same event, what each needs of that one
//!   delivery can hold together: the *event part* (conjuncts reading no
//!   `self.x`) of a rule's `when` and target filter, or of a subscription's
//!   `where` and guards (any of its transitions), must [`overlap`]. So
//!   `when: event.policy == sanctioned` and `target: Case where
//!   event.policy == unsanctioned and …` do not race.
//!
//! The last rule assumes the two deliveries come from one emission of the
//! event; two emissions of one event with different payloads inside one
//! cascade are not considered.
//!
//! Each pair is reported once, with the origin event closest to both: the
//! fewest transitions between the event and the two parties combined, ties
//! going to the event first in model order. Two rules give
//! [`FindingDetail::RaceCandidate`]; a pair with a subscription gives
//! [`FindingDetail::SubscriptionRace`].

use std::collections::BTreeMap;

use super::conditions::{data_condition, event_part};
use super::describe;
use super::{Finding, FindingDetail, RaceParty};
use crate::causal::{CausalGraph, CausalNode, Cone, Direction, NodeIx};
use crate::expr::{Condition, Expr, Overlap, overlap};
use crate::ids::{ControllerId, EventId, MachineId, TransitionId, TriggerId};
use crate::model::{Model, Target};

/// Which instances a party can reach, as far as aliasing goes.
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
            Target::One { filter: None } => InstanceScope::Singleton,
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

/// A party that can take part in a race, with what the check needs of it.
#[derive(Clone, Debug)]
struct Party {
    party: RaceParty,
    /// The controller of a rule; `None` for a subscription.
    controller: Option<ControllerId>,
    machine: MachineId,
    scope: InstanceScope,
    /// The causal node that reaching this party means: the rule's handler,
    /// or the subscription's event.
    node: NodeIx,
    /// The event it reacts to.
    event: EventId,
    /// What it needs of that event's payload; `None` when nothing is known.
    condition: Option<Expr>,
    /// The transitions it can make an instance take.
    transitions: Vec<TransitionId>,
}

/// The origin chosen for a pair so far.
#[derive(Clone, Copy, Debug)]
struct Origin {
    distance: u32,
    event: EventId,
}

pub(super) fn check(model: &Model, graph: &CausalGraph) -> Vec<Finding> {
    let parties = parties(model, graph);
    let downstream: Vec<Cone> = parties
        .iter()
        .map(|p| {
            let seeds: Vec<NodeIx> =
                p.transitions.iter().filter_map(|&t| graph.ix_of(CausalNode::Transition(t))).collect();
            graph.cone(&seeds, Direction::Forward, None)
        })
        .collect();
    let event_cones: Vec<Option<Cone>> = model
        .event_ids()
        .map(|e| graph.ix_of(CausalNode::Event(e)).map(|seed| graph.cone(&[seed], Direction::Forward, None)))
        .collect();
    // Parties by the node that reaches them.
    let mut by_node: BTreeMap<NodeIx, Vec<usize>> = BTreeMap::new();
    for (i, p) in parties.iter().enumerate() {
        by_node.entry(p.node).or_default().push(i);
    }

    let mut best: BTreeMap<(RaceParty, RaceParty), Origin> = BTreeMap::new();
    for event in model.event_ids() {
        let Some(cone) = &event_cones[event.index()] else {
            continue;
        };
        // (party index, hops from the event to the party's node)
        let mut reached: Vec<(usize, u32)> = cone
            .nodes()
            .flat_map(|(node, hops)| by_node.get(&node).into_iter().flatten().map(move |&i| (i, hops)))
            .collect();
        reached.sort_by_key(|&(i, _)| (parties[i].machine, parties[i].party));

        for group in reached.chunk_by(|a, b| parties[a.0].machine == parties[b.0].machine) {
            for (k, &(i, hops_a)) in group.iter().enumerate() {
                for &(j, hops_b) in &group[k + 1..] {
                    let (a, b) = (&parties[i], &parties[j]);
                    if !may_race(model, a, b)
                        || downstream[i].contains(b.node)
                        || downstream[j].contains(a.node)
                        || emission_ordered(model, &event_cones, a, b)
                    {
                        continue;
                    }
                    let candidate = Origin { distance: hops_a + hops_b, event };
                    best.entry((a.party, b.party))
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
            let machine = party_machine(model, first);
            let message = message(model, origin.event, first, second, machine);
            let detail = match (first, second) {
                (RaceParty::Rule(first), RaceParty::Rule(second)) => {
                    FindingDetail::RaceCandidate { origin: origin.event, machine, first, second }
                }
                _ => FindingDetail::SubscriptionRace { origin: origin.event, machine, first, second },
            };
            Finding::new(detail, message)
        })
        .collect()
}

/// Every rule and subscription that can race, rules first, each in model
/// order (so parties sort by `RaceParty`).
fn parties(model: &Model, graph: &CausalGraph) -> Vec<Party> {
    let mut out = Vec::new();
    for (rule, r) in model.rules() {
        let trigger = model.trigger(r.trigger);
        let scope = InstanceScope::of(&r.target);
        let Some(node) = graph.ix_of(CausalNode::Handler(r.handler)) else {
            continue;
        };
        if trigger.accepted_by.is_empty() || scope == InstanceScope::Fresh {
            continue;
        }
        let when = r.condition.as_ref().and_then(Condition::expr).and_then(event_part);
        let filter = r.target.filter().and_then(event_part);
        out.push(Party {
            party: RaceParty::Rule(rule),
            controller: Some(r.controller),
            machine: trigger.machine,
            scope,
            node,
            event: r.event,
            condition: Expr::and(when.into_iter().chain(filter).collect()),
            transitions: trigger.accepted_by.clone(),
        });
    }
    for (event, e) in model.events() {
        let Some(node) = graph.ix_of(CausalNode::Event(event)) else {
            continue;
        };
        for &t in &e.subscribers {
            let trigger = model.trigger(t);
            if trigger.accepted_by.is_empty() {
                continue;
            }
            out.push(Party {
                party: RaceParty::Subscription(t),
                controller: None,
                machine: trigger.machine,
                scope: InstanceScope::Selected,
                node,
                event,
                condition: subscription_condition(model, &trigger.accepted_by),
                transitions: trigger.accepted_by.clone(),
            });
        }
    }
    out
}

/// What a subscription needs of the event: the event part of any of its
/// transitions' `where` and guard. `None` when one of them needs nothing.
fn subscription_condition(model: &Model, transitions: &[TransitionId]) -> Option<Expr> {
    let parts: Option<Vec<Expr>> =
        transitions.iter().map(|&t| data_condition(model, t).as_ref().and_then(event_part)).collect();
    let mut parts = parts?;
    parts.dedup();
    Expr::or(parts)
}

/// The pair rules that do not need the causal graph: controllers, aliasing
/// and exclusive conditions on one delivery.
fn may_race(model: &Model, a: &Party, b: &Party) -> bool {
    if a.controller.is_some() && a.controller == b.controller {
        return false;
    }
    if !a.scope.may_alias(b.scope) {
        return false;
    }
    if a.event == b.event
        && let (Some(x), Some(y)) = (&a.condition, &b.condition)
        && overlap(x, y, &model.event_types(a.event)) == Overlap::Exclusive
    {
        return false;
    }
    true
}

/// Two subscriptions whose events reach each instance in a fixed order.
fn emission_ordered(model: &Model, event_cones: &[Option<Cone>], a: &Party, b: &Party) -> bool {
    let (RaceParty::Subscription(_), RaceParty::Subscription(_)) = (a.party, b.party) else {
        return false;
    };
    let emitters = |p: &Party| &model.event(p.event).emitted_by;
    if !emitters(a).is_empty() && emitters(a) == emitters(b) {
        return true;
    }
    let after = |earlier: &Party, later: &Party| {
        event_cones[earlier.event.index()].as_ref().is_some_and(|cone| cone.contains(later.node))
    };
    after(a, b) || after(b, a)
}

fn party_machine(model: &Model, party: RaceParty) -> MachineId {
    match party {
        RaceParty::Rule(rule) => model.trigger(model.rule(rule).trigger).machine,
        RaceParty::Subscription(trigger) => model.trigger(trigger).machine,
    }
}

/// `PaymentAuthorized leads Billing to fire Payment.capture and FraudCheck to
/// fire Payment.void, which may hit the same Payment instance in either
/// order`; a subscription reads `Payment to receive PaymentAuthorized`.
fn message(model: &Model, origin: EventId, first: RaceParty, second: RaceParty, machine: MachineId) -> String {
    let part = |party: RaceParty| match party {
        RaceParty::Rule(rule) => {
            let r = model.rule(rule);
            format!("{} to fire {}", model.controller(r.controller).name, describe::trigger(model, r.trigger))
        }
        RaceParty::Subscription(trigger) => subscription_part(model, trigger),
    };
    format!(
        "{} leads {} and {}, which may hit the same {} instance in either order",
        model.event(origin).name,
        part(first),
        part(second),
        model.machine(machine).name
    )
}

fn subscription_part(model: &Model, trigger: TriggerId) -> String {
    let t = model.trigger(trigger);
    format!("{} to receive {}", model.machine(t.machine).name, t.name)
}
