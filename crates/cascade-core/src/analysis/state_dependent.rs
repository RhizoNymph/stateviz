//! State-dependent fire (info): a rule's trigger is accepted somewhere in the
//! target machine, but not from every state the target can be in, so the
//! fire is dropped in the others.
//!
//! The states considered are the target machine's leaf states (atomic and
//! final; compound states are never current on their own, and history
//! pseudo-states are never current at all). A leaf drops the fire when
//! [`Model::enabled_transitions`] is empty for it, so transitions declared on
//! an ancestor count. A spawning rule (`new Machine …`) always lands in the
//! new instance's initial state (its default entry), so only that leaf is
//! considered for it. Rules whose trigger nothing accepts are invalid fires,
//! not reported here.

use super::describe;
use super::{Finding, FindingDetail};
use crate::ids::{RuleId, StateId};
use crate::model::{Model, Target};

pub(super) fn check(model: &Model) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (rule, r) in model.rules() {
        let trigger = model.trigger(r.trigger);
        if trigger.accepted_by.is_empty() {
            continue;
        }
        let machine = model.machine(trigger.machine);
        let candidates: Vec<StateId> = match r.target {
            Target::Spawn { .. } => vec![model.default_entry(machine.initial)],
            Target::One { .. } | Target::All { .. } => machine
                .states
                .iter()
                .copied()
                .filter(|&s| {
                    let s = model.state(s);
                    !s.is_compound() && !s.is_history()
                })
                .collect(),
        };
        let dropped_in: Vec<StateId> =
            candidates.into_iter().filter(|&s| model.enabled_transitions(s, r.trigger).is_empty()).collect();
        if !dropped_in.is_empty() {
            let message = message(model, rule, &dropped_in, matches!(r.target, Target::Spawn { .. }));
            findings.push(Finding::new(
                FindingDetail::StateDependentFire { rule, trigger: r.trigger, dropped_in },
                message,
            ));
        }
    }
    findings
}

fn message(model: &Model, rule: RuleId, dropped_in: &[StateId], spawns: bool) -> String {
    let r = model.rule(rule);
    let machine = &model.machine(model.trigger(r.trigger).machine).name;
    let paths: Vec<String> = dropped_in.iter().map(|&s| model.state(s).path.clone()).collect();
    if spawns {
        format!(
            "{}, but a new {machine} starts in {}, which drops it",
            describe::rule(model, rule),
            describe::list(&paths)
        )
    } else {
        format!("{}, which {machine} drops in {}", describe::rule(model, rule), describe::list(&paths))
    }
}
