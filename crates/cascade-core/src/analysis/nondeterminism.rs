//! Nondeterminism (error): one state declares two or more transitions on the
//! same trigger, and nothing guarantees that at most one of them applies.
//!
//! A group is ambiguous unless every pair of candidates is exclusive:
//!
//! - `else` is exclusive with everything (it applies only when nothing else
//!   does), but a group may have at most one;
//! - two candidates whose data conditions (subscription `where` filter and
//!   `Expr` guard, see [`conditions`](super::conditions)) cannot hold
//!   together ([`overlap`] is `Exclusive`) are exclusive, whatever else
//!   their guards say;
//! - two different `?choice` labels are exclusive (a scenario or player
//!   picks one);
//! - an `Expr` guard next to a `?choice` may hold while the choice is
//!   offered (mixed guards), and a candidate with no guard and no filter
//!   competes with every other one.
//!
//! The first reason found is reported, in this order: an unguarded
//! candidate, several `else`, a duplicated guard, then the first
//! overlapping pair in definition order. Transitions inherited from ancestor
//! states have lower priority (see [`Model::enabled_transitions`]) and never
//! conflict with a state's own transitions, so only transitions declared on
//! the same state are compared.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use super::conditions::{data_condition, transition_types};
use super::describe;
use super::{Finding, FindingDetail};
use crate::expr::{Coverage, Expr, Guard, Overlap, coverage, overlap};
use crate::ids::{StateId, TransitionId, TriggerId};
use crate::model::Model;

/// Why a group of transitions on one state and trigger is ambiguous.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ambiguity {
    /// This candidate has no guard and no filter, so it competes with every
    /// other candidate.
    Unguarded(TransitionId),
    /// More than one candidate is guarded by `else`.
    SeveralElse(usize),
    /// Two or more candidates share this guard (and filter).
    DuplicateGuard { guard: String, times: usize },
    /// An expression guard may hold next to a `?choice`.
    Mixed { expr: String, choice: String },
    /// Two data conditions can hold together; `example` is a scope where
    /// both do, when the solver found one.
    Overlap { first: String, second: String, example: Option<String> },
}

/// One candidate of a group, as the check sees it.
struct Candidate<'m> {
    transition: TransitionId,
    guard: Option<&'m Guard>,
    filter: Option<&'m Expr>,
    /// `filter and guard` (expression guards only).
    data: Option<Expr>,
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
pub(super) fn group_by_source(
    model: &Model,
    transitions: &[TransitionId],
) -> Vec<((StateId, TriggerId), Vec<TransitionId>)> {
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

fn ambiguity(model: &Model, transitions: &[TransitionId]) -> Option<Ambiguity> {
    let candidates: Vec<Candidate<'_>> = transitions
        .iter()
        .map(|&t| {
            let tr = model.transition(t);
            Candidate {
                transition: t,
                guard: tr.guard.as_ref(),
                filter: tr.filter.as_ref(),
                data: data_condition(model, t),
            }
        })
        .collect();

    if let Some(c) = candidates.iter().find(|c| c.guard.is_none() && c.filter.is_none()) {
        return Some(Ambiguity::Unguarded(c.transition));
    }

    let elses = candidates.iter().filter(|c| c.guard == Some(&Guard::Else)).count();
    if elses > 1 {
        return Some(Ambiguity::SeveralElse(elses));
    }

    let same = |a: &Candidate<'_>, b: &Candidate<'_>| a.guard == b.guard && a.filter == b.filter;
    for (i, c) in candidates.iter().enumerate() {
        let times = candidates.iter().filter(|o| same(c, o)).count();
        if times > 1 && !candidates[..i].iter().any(|o| same(c, o)) {
            let guard =
                c.guard.map_or_else(|| c.filter.map(ToString::to_string).unwrap_or_default(), ToString::to_string);
            return Some(Ambiguity::DuplicateGuard { guard, times });
        }
    }

    // Every candidate of a group shares the trigger, so one set of types
    // reads them all.
    let types = transition_types(model, transitions[0]);
    for (i, a) in candidates.iter().enumerate() {
        for b in &candidates[i + 1..] {
            if let Some(reason) = pair_ambiguity(a, b, &types) {
                return Some(reason);
            }
        }
    }
    None
}

/// Why `a` and `b` may both apply, or `None` when they are exclusive.
fn pair_ambiguity(a: &Candidate<'_>, b: &Candidate<'_>, types: &crate::expr::Types) -> Option<Ambiguity> {
    if a.guard == Some(&Guard::Else) || b.guard == Some(&Guard::Else) {
        return None;
    }
    if let (Some(x), Some(y)) = (&a.data, &b.data)
        && overlap(x, y, types) == Overlap::Exclusive
    {
        return None;
    }
    match (a.guard, b.guard) {
        (Some(Guard::Choice(x)), Some(Guard::Choice(y))) if x != y => None,
        (Some(Guard::Expr(e)), Some(Guard::Choice(label))) | (Some(Guard::Choice(label)), Some(Guard::Expr(e))) => {
            Some(Ambiguity::Mixed { expr: e.to_string(), choice: format!("?{label}") })
        }
        _ => {
            // Data that may hold together, or the same choice under filters
            // that may hold together.
            let example = match (&a.data, &b.data) {
                (Some(x), Some(y)) => example(x, y, types),
                _ => None,
            };
            Some(Ambiguity::Overlap { first: a.text(), second: b.text(), example })
        }
    }
}

impl Candidate<'_> {
    /// How the candidate's condition reads in messages: its data condition,
    /// else its guard.
    fn text(&self) -> String {
        match (&self.data, self.guard) {
            (Some(data), Some(Guard::Choice(label))) => format!("{data} and ?{label}"),
            (Some(data), _) => data.to_string(),
            (None, Some(guard)) => guard.to_string(),
            (None, None) => String::new(),
        }
    }
}

/// A readable scope where both `a` and `b` hold, when the solver finds one:
/// a gap in covering `not (a and b)`.
fn example(a: &Expr, b: &Expr, types: &crate::expr::Types) -> Option<String> {
    let neither = Expr::And(vec![a.clone(), b.clone()]).negate();
    match coverage(&[&neither], types) {
        Coverage::Gaps(examples) => examples.into_iter().next(),
        Coverage::Complete | Coverage::Unknown => None,
    }
}

fn message(model: &Model, state: StateId, trigger: TriggerId, n: usize, ambiguity: &Ambiguity) -> String {
    let reason = match ambiguity {
        Ambiguity::Unguarded(t) => format!("{} has no guard", model.transition_label(*t)),
        Ambiguity::SeveralElse(k) => format!("{k} of them are guarded by `else`"),
        Ambiguity::DuplicateGuard { guard, times } => format!("the guard `{guard}` appears {times} times"),
        Ambiguity::Mixed { expr, choice } => {
            format!("mixed guards: `{expr}` may hold while the choice `{choice}` is offered")
        }
        Ambiguity::Overlap { first, second, example: Some(example) } => {
            format!("`{first}` and `{second}` both hold when {example}")
        }
        Ambiguity::Overlap { first, second, example: None } => format!("`{first}` and `{second}` may both hold"),
    };
    format!(
        "{} has {n} transitions on `{}` without mutually exclusive guards: {reason}",
        describe::state(model, state),
        model.trigger(trigger).name
    )
}
