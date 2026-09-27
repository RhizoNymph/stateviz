//! Replaying a race candidate in both orders.
//!
//! 1. Run the scenario in FIFO order.
//! 2. Find the contested pair in that trace: a fire of each of the finding's
//!    two rules, at the same instance, both caused (through any chain of
//!    steps) by the same emission of the finding's origin event. When several
//!    pairs qualify, the one delivered earliest wins.
//! 3. Run again with the pair swapped (see `engine::schedule`): the fire FIFO
//!    delivered first yields to the other.
//!
//! Both runs are deterministic, so the fire to hold back is identified in the
//! second run by rule, instance name and occurrence count.

use std::collections::{BTreeSet, HashMap};

use cascade_core::ids::{EventId, RuleId};
use cascade_core::{ElementRef, FindingDetail, Model};

use crate::engine::{self, FireKey, Swap};
use crate::error::SimError;
use crate::scenario::{Scenario, validate};
use crate::trace::{Lifeline, LifelineIx, Trace, TraceStepKind};
use crate::{RaceRuns, SimRun};

pub(crate) fn race_runs(model: &Model, scenario: &Scenario, race: &FindingDetail) -> Result<RaceRuns, SimError> {
    let FindingDetail::RaceCandidate { origin, first, second, .. } = race else {
        return Err(SimError::NotARace);
    };
    let resolved = validate(model, scenario)?;
    let mut as_queued: SimRun = engine::run(model, &resolved, None)?.run;

    let Some(pair) = contested_pair(&as_queued.trace, *origin, *first, *second) else {
        return Err(SimError::RaceNotReached {
            origin: model.event(*origin).name.clone(),
            first: rule_label(model, *first),
            second: rule_label(model, *second),
        });
    };
    tracing::debug!(
        instance = %pair.instance,
        earlier = %rule_label(model, pair.earlier.rule),
        later = %rule_label(model, pair.later.rule),
        "replaying race with the contested fires swapped"
    );

    let swap = Swap {
        yielder: FireKey { rule: pair.earlier.rule, instance: pair.instance, occurrence: pair.earlier.occurrence },
        overtaker: pair.later.rule,
    };
    let outcome = engine::run(model, &resolved, Some(swap))?;
    if !outcome.swapped {
        return Err(SimError::RaceNotSwappable {
            earlier: rule_label(model, pair.earlier.rule),
            later: rule_label(model, pair.later.rule),
        });
    }

    let (earlier_label, later_label) = ordering_labels(model, pair.earlier.rule, pair.later.rule);
    as_queued.trace.ordering = Some(earlier_label);
    let mut swapped = outcome.run;
    swapped.trace.ordering = Some(later_label);
    Ok(RaceRuns { as_queued, swapped })
}

fn rule_label(model: &Model, rule: RuleId) -> String {
    model.label_of(ElementRef::Rule(rule))
}

/// "Fulfillment first" / "Billing first"; the full rule labels when both
/// rules belong to one controller.
fn ordering_labels(model: &Model, earlier: RuleId, later: RuleId) -> (String, String) {
    let controller = |rule: RuleId| model.rule(rule).controller;
    let name = |rule: RuleId| {
        if controller(earlier) == controller(later) {
            rule_label(model, rule)
        } else {
            model.controller(controller(rule)).name.clone()
        }
    };
    (format!("{} first", name(earlier)), format!("{} first", name(later)))
}

#[derive(Clone, Debug)]
struct ContestedFire {
    rule: RuleId,
    /// Fires of the same rule at the same instance before this one.
    occurrence: u32,
    /// Index of the step that delivered it (its transition or drop).
    delivered: usize,
}

#[derive(Clone, Debug)]
struct Pair {
    instance: String,
    earlier: ContestedFire,
    later: ContestedFire,
}

fn contested_pair(trace: &Trace, origin: EventId, first: RuleId, second: RuleId) -> Option<Pair> {
    // Fire step → the step that delivered it.
    let mut delivered: HashMap<usize, usize> = HashMap::new();
    for (i, step) in trace.steps.iter().enumerate() {
        if matches!(step.kind, TraceStepKind::Transition { .. } | TraceStepKind::Dropped { .. })
            && let Some(cause) = step.cause
        {
            delivered.entry(cause.index()).or_insert(i);
        }
    }

    let mut counts: HashMap<(RuleId, LifelineIx), u32> = HashMap::new();
    let mut fires: Vec<(LifelineIx, ContestedFire, BTreeSet<usize>)> = Vec::new();
    for (i, step) in trace.steps.iter().enumerate() {
        let TraceStepKind::Fire { target, rule, .. } = step.kind else {
            continue;
        };
        let count = counts.entry((rule, target)).or_insert(0);
        let occurrence = *count;
        *count += 1;
        if rule != first && rule != second {
            continue;
        }
        if let Some(&at) = delivered.get(&i) {
            fires.push((target, ContestedFire { rule, occurrence, delivered: at }, origin_emissions(trace, i, origin)));
        }
    }

    let mut best: Option<((usize, usize), usize, usize)> = None;
    for (ai, (a_target, a, a_origins)) in fires.iter().enumerate() {
        for (bi, (b_target, b, b_origins)) in fires.iter().enumerate().skip(ai + 1) {
            let contested = (a.rule == first && b.rule == second) || (a.rule == second && b.rule == first);
            if !contested || a_target != b_target || a_origins.is_disjoint(b_origins) {
                continue;
            }
            let key = (a.delivered.min(b.delivered), a.delivered.max(b.delivered));
            if best.as_ref().is_none_or(|(k, ..)| key < *k) {
                best = Some((key, ai, bi));
            }
        }
    }

    let (_, ai, bi) = best?;
    let (target, a, _) = fires.get(ai)?;
    let (_, b, _) = fires.get(bi)?;
    let Some(Lifeline::Instance { name, .. }) = trace.lifelines.get(target.index()) else {
        return None;
    };
    let (earlier, later) = if a.delivered < b.delivered { (a, b) } else { (b, a) };
    Some(Pair { instance: name.clone(), earlier: earlier.clone(), later: later.clone() })
}

/// Emit steps of `origin` among the causes of `step`.
fn origin_emissions(trace: &Trace, step: usize, origin: EventId) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    let mut current = step;
    let mut cursor = trace.steps.get(step).and_then(|s| s.cause);
    while let Some(ix) = cursor {
        // Causes always point backwards; stop on anything else.
        let Some(cause) = trace.steps.get(ix.index()).filter(|_| ix.index() < current) else {
            break;
        };
        current = ix.index();
        if matches!(cause.kind, TraceStepKind::Emit { event, .. } if event == origin) {
            out.insert(ix.index());
        }
        cursor = cause.cause;
    }
    out
}
