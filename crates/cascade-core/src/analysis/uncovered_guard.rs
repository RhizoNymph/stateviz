//! Uncovered guard (info): a state's transitions on a trigger are all
//! guarded by expressions, there is no `else`, and some input makes every
//! guard false, so the trigger is silently dropped there.
//!
//! Groups are the nondeterminism check's `(from, trigger)` groups, single
//! transitions included. A group is considered only when every candidate has
//! an `Expr` guard (a `?choice`, `else` or unguarded candidate means the
//! designer already decided what happens otherwise). [`coverage`] decides
//! whether the guards always leave one holding:
//!
//! - Command and timer triggers: coverage of the guards.
//! - Subscriptions: a delivery whose `where` filter fails is ignored by
//!   design (the instance is not interested), so only inputs that pass some
//!   filter count. When every candidate has the same filter (or none),
//!   coverage of the guards alone; otherwise coverage of each candidate's
//!   `filter and guard` together with `not (filter₁ or filter₂ …)`.
//!
//! A gap example made only of missing fields (`event.amount is missing`)
//! is dropped: a missing value is a data error, not a design gap. When no
//! example remains, or coverage is `Unknown`, nothing is reported.

use super::conditions::transition_types;
use super::describe;
use super::nondeterminism::group_by_source;
use super::{Finding, FindingDetail};
use crate::expr::{Coverage, Expr, Guard, coverage};
use crate::ids::{StateId, TransitionId, TriggerId};
use crate::model::Model;

pub(super) fn check(model: &Model) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (_, machine) in model.machines() {
        for ((state, trigger), transitions) in group_by_source(model, &machine.transitions) {
            let Some(gaps) = gaps(model, &transitions) else {
                continue;
            };
            let message = message(model, state, trigger, &gaps);
            findings.push(Finding::new(FindingDetail::UncoveredGuard { state, trigger, transitions, gaps }, message));
        }
    }
    findings
}

/// Readable inputs that drop the trigger, or `None` when the group is not
/// all-expression, is covered, or the solver cannot tell.
fn gaps(model: &Model, transitions: &[TransitionId]) -> Option<Vec<String>> {
    let mut guards: Vec<&Expr> = Vec::with_capacity(transitions.len());
    for &t in transitions {
        match &model.transition(t).guard {
            Some(Guard::Expr(expr)) => guards.push(expr),
            Some(Guard::Choice(_) | Guard::Else) | None => return None,
        }
    }
    let filters: Vec<Option<&Expr>> = transitions.iter().map(|&t| model.transition(t).filter.as_ref()).collect();
    let types = transition_types(model, transitions[0]);

    let result = if filters.windows(2).all(|w| w[0] == w[1]) {
        coverage(&guards, &types)
    } else {
        let mut exprs: Vec<Expr> = filters
            .iter()
            .zip(&guards)
            .map(|(filter, guard)| match filter {
                Some(filter) => Expr::And(vec![(*filter).clone(), (*guard).clone()]),
                None => (*guard).clone(),
            })
            .collect();
        // Inputs no filter accepts are ignored, not dropped. (A candidate
        // without a filter accepts every input, so then nothing is excluded.)
        if filters.iter().all(Option::is_some) {
            let any_filter = Expr::or(filters.iter().flatten().map(|f| (*f).clone()).collect());
            exprs.extend(any_filter.map(Expr::negate));
        }
        coverage(&exprs.iter().collect::<Vec<_>>(), &types)
    };
    match result {
        Coverage::Gaps(examples) => {
            let examples: Vec<String> = examples.into_iter().filter(|e| !only_missing(e)).collect();
            (!examples.is_empty()).then_some(examples)
        }
        Coverage::Complete | Coverage::Unknown => None,
    }
}

/// Whether every part of a gap example (`self.a is missing, event.b is
/// missing`) is a missing field.
fn only_missing(example: &str) -> bool {
    example.split(", ").all(|part| part.ends_with(" is missing"))
}

/// `Order.pending drops capture when event.method = wire; or when …`.
fn message(model: &Model, state: StateId, trigger: TriggerId, gaps: &[String]) -> String {
    format!(
        "{} drops `{}` when {}: no guard holds and there is no `else`",
        describe::state(model, state),
        model.trigger(trigger).name,
        gaps.join("; or when ")
    )
}

#[cfg(test)]
mod tests {
    use super::only_missing;

    #[test]
    fn missing_only_examples_are_recognized() {
        assert!(only_missing("event.amount is missing"));
        assert!(only_missing("self.a is missing, event.b is missing"));
        assert!(!only_missing("self.a = x, event.b is missing"));
        assert!(!only_missing("event.method = wire"));
    }
}
