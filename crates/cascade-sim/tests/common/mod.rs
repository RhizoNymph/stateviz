//! Shared helpers for the simulator's integration tests.

#![allow(dead_code)]

use cascade_core::{Model, load_str};
use cascade_sim::{
    Lifeline, LifelineIx, Scenario, ScenarioError, ScenarioErrorKind, SimError, Trace, TraceStepKind, parse_scenario,
    simulate,
};

pub fn model(text: &str) -> Model {
    match load_str(text) {
        Ok(model) => model,
        Err(err) => panic!("expected the definition to load:\n{err}"),
    }
}

pub fn scenario(text: &str) -> Scenario {
    match parse_scenario(text) {
        Ok(scenario) => scenario,
        Err(err) => panic!("expected the scenario to parse:\n{err}"),
    }
}

pub fn scenario_err(text: &str) -> ScenarioError {
    match parse_scenario(text) {
        Ok(scenario) => panic!("expected scenario diagnostics, got {scenario:?}"),
        Err(err) => err,
    }
}

pub fn has(err: &ScenarioError, pred: impl Fn(&ScenarioErrorKind) -> bool) -> bool {
    err.diagnostics.iter().any(|d| pred(&d.kind))
}

pub fn run(model: &Model, scenario_text: &str) -> Trace {
    match simulate(model, &scenario(scenario_text)) {
        Ok(trace) => trace,
        Err(err) => panic!("expected the scenario to run:\n{err}"),
    }
}

pub fn run_err(model: &Model, scenario_text: &str) -> SimError {
    match simulate(model, &scenario(scenario_text)) {
        Ok(trace) => panic!("expected the run to fail, got {} steps", trace.steps.len()),
        Err(err) => err,
    }
}

/// The name of a lifeline: the instance name, source or controller name.
pub fn name(model: &Model, trace: &Trace, ix: LifelineIx) -> String {
    match &trace.lifelines[ix.index()] {
        Lifeline::External { source } => model.external(*source).name.clone(),
        Lifeline::Instance { name, .. } => name.clone(),
        Lifeline::Controller { controller } => model.controller(*controller).name.clone(),
    }
}

/// A compact, test-only rendering of each step, independent of the
/// library's display text.
pub fn lines(model: &Model, trace: &Trace) -> Vec<String> {
    let n = |ix| name(model, trace, ix);
    trace
        .steps
        .iter()
        .map(|step| match &step.kind {
            TraceStepKind::ExternalFire { source, target, trigger } => {
                format!("ext {} -> {} {}", n(*source), n(*target), model.trigger(*trigger).name)
            }
            TraceStepKind::Transition { instance, from, to, .. } => {
                format!("{} {} -> {}", n(*instance), model.state(*from).path, model.state(*to).path)
            }
            TraceStepKind::Dropped { instance, trigger, state } => {
                format!("{} drop {} @ {}", n(*instance), model.trigger(*trigger).name, model.state(*state).path)
            }
            TraceStepKind::Emit { instance, event } => format!("{} emit {}", n(*instance), model.event(*event).name),
            TraceStepKind::Deliver { controller, event, .. } => {
                format!("{} <- {}", n(*controller), model.event(*event).name)
            }
            TraceStepKind::Fire { controller, target, rule } => {
                format!("{} fire {} {}", n(*controller), n(*target), model.trigger(model.rule(*rule).trigger).name)
            }
            TraceStepKind::Spawn { controller, instance, .. } => format!("{} spawn {}", n(*controller), n(*instance)),
            TraceStepKind::NoTarget { controller, rule } => {
                format!("{} no-target {}", n(*controller), model.trigger(model.rule(*rule).trigger).name)
            }
            TraceStepKind::Ambiguous { controller, rule, candidates } => format!(
                "{} ambiguous {} [{}]",
                n(*controller),
                model.trigger(model.rule(*rule).trigger).name,
                candidates.iter().map(|&c| n(c)).collect::<Vec<_>>().join(", ")
            ),
        })
        .collect()
}

/// Each step's cause as a plain index.
pub fn causes(trace: &Trace) -> Vec<Option<u32>> {
    trace.steps.iter().map(|s| s.cause.map(|c| c.0)).collect()
}

/// `instance: state path` for every final state, in lifeline order.
pub fn finals(model: &Model, trace: &Trace) -> Vec<String> {
    trace
        .final_states
        .iter()
        .map(|(ix, state)| format!("{}: {}", name(model, trace, *ix), model.state(*state).path))
        .collect()
}

/// Lifelines as `ext:Name`, `Machine:name`, `ctl:Name`.
pub fn lifelines(model: &Model, trace: &Trace) -> Vec<String> {
    trace
        .lifelines
        .iter()
        .map(|l| match l {
            Lifeline::External { source } => format!("ext:{}", model.external(*source).name),
            Lifeline::Instance { machine, name } => format!("{}:{name}", model.machine(*machine).name),
            Lifeline::Controller { controller } => format!("ctl:{}", model.controller(*controller).name),
        })
        .collect()
}

pub fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}
