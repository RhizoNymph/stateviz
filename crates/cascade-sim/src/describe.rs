//! Plain-text descriptions of lifelines and trace steps, for the CLI and any
//! host that lists a trace as text.

use cascade_core::Model;
use cascade_core::definition::{TargetMode, TargetSpec};
use cascade_core::ids::RuleId;
use cascade_core::model::Target;

use crate::scenario::Payload;
use crate::trace::{Lifeline, LifelineIx, Trace, TraceStep, TraceStepKind};

/// `Customer`, `Order o1` or `Fulfillment`.
pub fn lifeline_label(model: &Model, lifeline: &Lifeline) -> String {
    match lifeline {
        Lifeline::External { source } => model.external(*source).name.clone(),
        Lifeline::Instance { machine, name } => format!("{} {name}", model.machine(*machine).name),
        Lifeline::Controller { controller } => model.controller(*controller).name.clone(),
    }
}

/// One line describing `step`, e.g. `Order o1: pending → paid (capture_ok)`
/// or `o1 emits OrderPaid {orderId: 1}`. `payload` is the step's payload
/// from [`SimRun::payloads`](crate::SimRun::payloads), if any.
pub fn step_text(model: &Model, trace: &Trace, step: &TraceStep, payload: Option<&Payload>) -> String {
    let who = |ix: LifelineIx| match trace.lifelines.get(ix.index()) {
        Some(Lifeline::Instance { name, .. }) => name.clone(),
        Some(other) => lifeline_label(model, other),
        None => "?".to_owned(),
    };
    let machine_of = |ix: LifelineIx| match trace.lifelines.get(ix.index()) {
        Some(Lifeline::Instance { machine, .. }) => model.machine(*machine).name.clone(),
        _ => "?".to_owned(),
    };
    let with_payload = |text: String| match payload {
        Some(p) => format!("{text} {}", payload_text(p)),
        None => text,
    };
    let trigger_of = |rule: RuleId| model.trigger(model.rule(rule).trigger).name.clone();

    match &step.kind {
        TraceStepKind::ExternalFire { source, target, trigger } => {
            with_payload(format!("{} fires {} at {}", who(*source), model.trigger(*trigger).name, who(*target)))
        }
        TraceStepKind::Transition { instance, transition, from, to } => format!(
            "{} {}: {} → {} ({})",
            machine_of(*instance),
            who(*instance),
            model.state(*from).path,
            model.state(*to).path,
            model.trigger(model.transition(*transition).trigger).name
        ),
        TraceStepKind::Dropped { instance, trigger, state } => format!(
            "{} {}: {} dropped in {}",
            machine_of(*instance),
            who(*instance),
            model.trigger(*trigger).name,
            model.state(*state).path
        ),
        TraceStepKind::Emit { instance, event } => {
            with_payload(format!("{} emits {}", who(*instance), model.event(*event).name))
        }
        TraceStepKind::Deliver { controller, event, .. } => {
            format!("{} receives {}", who(*controller), model.event(*event).name)
        }
        TraceStepKind::Fire { controller, target, rule } => {
            format!("{} fires {} at {}", who(*controller), trigger_of(*rule), who(*target))
        }
        TraceStepKind::Spawn { controller, instance, .. } => {
            format!("{} spawns {} {}", who(*controller), machine_of(*instance), who(*instance))
        }
        TraceStepKind::NoTarget { controller, rule } => {
            format!("{}: no target for {} ({})", who(*controller), trigger_of(*rule), selector_text(model, *rule))
        }
        TraceStepKind::Ambiguous { controller, rule, candidates } => format!(
            "{}: {} not fired, {} targets match ({}) ({})",
            who(*controller),
            trigger_of(*rule),
            candidates.len(),
            candidates.iter().map(|&c| who(c)).collect::<Vec<_>>().join(", "),
            selector_text(model, *rule)
        ),
    }
}

/// `{amount: 42, orderId: 1}`; values that are not plain words are quoted.
pub fn payload_text(payload: &Payload) -> String {
    let entries: Vec<String> = payload
        .iter()
        .map(|(k, v)| {
            let plain = !v.is_empty() && v.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'));
            if plain { format!("{k}: {v}") } else { format!("{k}: {v:?}") }
        })
        .collect();
    format!("{{{}}}", entries.join(", "))
}

/// A rule's target selector as written, e.g. `Shipment where orderId ==
/// event.orderId`.
pub fn selector_text(model: &Model, rule: RuleId) -> String {
    let r = model.rule(rule);
    let machine = model.machine(model.trigger(r.trigger).machine).name.clone();
    let (mode, clauses) = match &r.target {
        Target::One { predicates } => (TargetMode::One, predicates),
        Target::All { predicates } => (TargetMode::All, predicates),
        Target::Spawn { assignments } => (TargetMode::Spawn, assignments),
    };
    TargetSpec { mode, machine, clauses: clauses.clone() }.to_string()
}

/// Each step's causal depth: 0 for scenario steps, one more than its cause
/// otherwise.
pub fn causal_depths(trace: &Trace) -> Vec<usize> {
    let mut depths: Vec<usize> = Vec::with_capacity(trace.steps.len());
    for (i, step) in trace.steps.iter().enumerate() {
        let depth = step.cause.filter(|c| c.index() < i).and_then(|c| depths.get(c.index())).map_or(0, |d| d + 1);
        depths.push(depth);
    }
    depths
}
