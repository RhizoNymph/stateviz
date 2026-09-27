//! Nondeterminism (error): one state declares two or more transitions on the
//! same trigger, and their guards do not tell them apart.
//!
//! Guards are free text and never evaluated, so "mutually exclusive" is
//! approximated syntactically: every candidate has a guard, the guards are
//! pairwise distinct after whitespace normalization, and at most one of them
//! is `else`. Transitions inherited from ancestor states have lower priority
//! (see [`Model::enabled_transitions`]) and never conflict with a state's
//! own transitions, so only transitions declared on the same state are
//! compared.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use super::describe;
use super::{Finding, FindingDetail};
use crate::ids::{StateId, TransitionId, TriggerId};
use crate::model::Model;

/// Why a group of transitions on one state and trigger is ambiguous. The
/// first applicable reason is reported.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ambiguity {
    /// This candidate has no guard (or a blank one), so it competes with
    /// every other candidate.
    Unguarded(TransitionId),
    /// More than one candidate is guarded by `else`.
    SeveralElse(usize),
    /// Two or more candidates share this (normalized) guard.
    DuplicateGuard { guard: String, times: usize },
}

pub(super) fn check(model: &Model) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (_, machine) in model.machines() {
        for ((state, trigger), transitions) in group_by_source(model, &machine.transitions) {
            if transitions.len() < 2 {
                continue;
            }
            if let Some(ambiguity) = ambiguity(model, &transitions) {
                let message = message(model, state, trigger, transitions.len(), &ambiguity);
                findings.push(Finding::new(FindingDetail::Nondeterminism { state, trigger, transitions }, message));
            }
        }
    }
    findings
}

/// Transitions grouped by `(from, trigger)`, groups in order of first
/// appearance, members in model order.
fn group_by_source(model: &Model, transitions: &[TransitionId]) -> Vec<((StateId, TriggerId), Vec<TransitionId>)> {
    let mut groups: Vec<((StateId, TriggerId), Vec<TransitionId>)> = Vec::new();
    let mut index: HashMap<(StateId, TriggerId), usize> = HashMap::new();
    for &t in transitions {
        let tr = model.transition(t);
        let key = (tr.from, tr.trigger);
        match index.entry(key) {
            Entry::Occupied(slot) => groups[*slot.get()].1.push(t),
            Entry::Vacant(slot) => {
                slot.insert(groups.len());
                groups.push((key, vec![t]));
            }
        }
    }
    groups
}

/// Collapse runs of whitespace to one space and trim, so guards that differ
/// only in spacing compare equal.
pub(super) fn normalize_guard(guard: &str) -> String {
    guard.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn ambiguity(model: &Model, transitions: &[TransitionId]) -> Option<Ambiguity> {
    let mut guards = Vec::with_capacity(transitions.len());
    for &t in transitions {
        let guard = model.transition(t).guard.as_deref().map(normalize_guard).unwrap_or_default();
        if guard.is_empty() {
            return Some(Ambiguity::Unguarded(t));
        }
        guards.push(guard);
    }

    let elses = guards.iter().filter(|g| *g == "else").count();
    if elses > 1 {
        return Some(Ambiguity::SeveralElse(elses));
    }

    guards.iter().enumerate().find_map(|(i, guard)| {
        let times = guards.iter().filter(|g| *g == guard).count();
        // Report each duplicated guard at its first occurrence only.
        (times > 1 && !guards[..i].contains(guard)).then(|| Ambiguity::DuplicateGuard { guard: guard.clone(), times })
    })
}

fn message(model: &Model, state: StateId, trigger: TriggerId, n: usize, ambiguity: &Ambiguity) -> String {
    let reason = match ambiguity {
        Ambiguity::Unguarded(t) => format!("{} has no guard", model.transition_label(*t)),
        Ambiguity::SeveralElse(k) => format!("{k} of them are guarded by `else`"),
        Ambiguity::DuplicateGuard { guard, times } => format!("the guard `{guard}` appears {times} times"),
    };
    format!(
        "{} has {n} transitions on `{}` without mutually exclusive guards: {reason}",
        describe::state(model, state),
        model.trigger(trigger).name
    )
}

#[cfg(test)]
mod tests {
    use super::normalize_guard;

    #[test]
    fn guard_normalization_collapses_whitespace() {
        assert_eq!(normalize_guard("  amount   >\t0 "), "amount > 0");
        assert_eq!(normalize_guard("else"), "else");
        assert_eq!(normalize_guard("   "), "");
        assert_ne!(normalize_guard("amount>0"), normalize_guard("amount > 0"));
    }
}
