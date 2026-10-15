//! Helpers for the exploration tests: running an exploration, checking that
//! every witness replays to exactly its outcome, and comparing outcome
//! sets.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use cascade_core::{ElementRef, Model, load_str};
use cascade_sim::{
    Anomaly, Exploration, ExploreOptions, Lifeline, OrderingModel, Outcome, OutcomeTimer, PlaySession, Scenario,
    TraceStepKind, explore, parse_scenario, replay_witness,
};

pub const RACE: &str = include_str!("../fixtures/race/cascade.yaml");
pub const RACE_SCENARIO: &str = include_str!("../fixtures/race/scenario.yaml");
pub const ORDERS: &str = include_str!("../../../../examples/order-fulfillment/cascade.yaml");
pub const ORDERS_SCENARIOS: [&str; 4] = [
    include_str!("../../../../examples/order-fulfillment/scenarios/happy-path.yaml"),
    include_str!("../../../../examples/order-fulfillment/scenarios/second-order-first.yaml"),
    include_str!("../../../../examples/order-fulfillment/scenarios/timeout-first.yaml"),
    include_str!("../../../../examples/order-fulfillment/scenarios/timeout-race.yaml"),
];
pub const SAGA: &str = include_str!("../../../../examples/checkout-saga/cascade.yaml");
pub const SAGA_SCENARIOS: [&str; 3] = [
    include_str!("../../../../examples/checkout-saga/scenarios/happy-path.yaml"),
    include_str!("../../../../examples/checkout-saga/scenarios/declined.yaml"),
    include_str!("../../../../examples/checkout-saga/scenarios/timeout.yaml"),
];
pub const SHOP: &str = include_str!("../../../../examples/shop/cascade.yaml");
/// A shop order from checkout to delivery (the example ships no scenario).
pub const SHOP_SCENARIO: &str = include_str!("../fixtures/explore/shop-order.yaml");

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

/// Every (definition, scenario) pair the examples and fixtures have.
pub fn corpus() -> Vec<(&'static str, &'static str)> {
    let mut out = vec![(RACE, RACE_SCENARIO), (SHOP, SHOP_SCENARIO)];
    out.extend(ORDERS_SCENARIOS.iter().map(|s| (ORDERS, *s)));
    out.extend(SAGA_SCENARIOS.iter().map(|s| (SAGA, *s)));
    out
}

/// Every combination of the options (reduction on).
pub fn option_grid() -> Vec<ExploreOptions> {
    let mut out = Vec::new();
    for ordering in [OrderingModel::PerConsumer, OrderingModel::Unordered] {
        for choices in [false, true] {
            for timers_anytime in [false, true] {
                out.push(ExploreOptions { ordering, choices, timers_anytime, ..ExploreOptions::default() });
            }
        }
    }
    out
}

pub fn run(model: &Model, scenario_text: &str, options: &ExploreOptions) -> Exploration {
    match explore(model, &scenario(scenario_text), options) {
        Ok(exploration) => exploration,
        Err(err) => panic!("expected the exploration to run:\n{err}"),
    }
}

/// An outcome without its witness and path count: what makes it distinct.
pub type Identity =
    (BTreeMap<String, (cascade_core::ids::StateId, BTreeMap<String, String>)>, Vec<OutcomeTimer>, BTreeSet<Anomaly>);

pub fn identity(outcome: &Outcome) -> Identity {
    (outcome.instances.clone(), outcome.timers.clone(), outcome.anomalies.clone())
}

pub fn identities(exploration: &Exploration) -> BTreeSet<Identity> {
    exploration.outcomes.iter().map(identity).collect()
}

/// `name: state` for each instance of an outcome, by name.
pub fn states(model: &Model, outcome: &Outcome) -> Vec<String> {
    outcome.instances.iter().map(|(name, (state, _))| format!("{name}: {}", model.state(*state).path)).collect()
}

pub fn anomaly_texts(outcome: &Outcome) -> Vec<String> {
    outcome.anomalies.iter().map(ToString::to_string).collect()
}

/// The anomalies a replayed session shows from step `from` on, as
/// exploration reports them (inputs skipped and the step limit leave no
/// trace, so they are not included).
fn replayed_anomalies(model: &Model, session: &PlaySession, from: usize) -> BTreeSet<Anomaly> {
    let trace = session.trace();
    let name = |ix: cascade_sim::LifelineIx| match &trace.lifelines[ix.index()] {
        Lifeline::Instance { name, .. } => name.clone(),
        other => format!("{other:?}"),
    };
    let rule = |r| model.label_of(ElementRef::Rule(r));
    let mut out: BTreeSet<Anomaly> = trace.steps[from..]
        .iter()
        .filter_map(|step| match &step.kind {
            TraceStepKind::NoTarget { rule: r, .. } => Some(Anomaly::NoTarget { rule: rule(*r) }),
            TraceStepKind::Ambiguous { rule: r, candidates, .. } => {
                Some(Anomaly::Ambiguous { rule: rule(*r), candidates: candidates.iter().map(|&c| name(c)).collect() })
            }
            TraceStepKind::Dropped { instance, trigger, state } => Some(Anomaly::Dropped {
                instance: name(*instance),
                trigger: model.trigger(*trigger).name.clone(),
                state: model.state(*state).path.clone(),
            }),
            _ => None,
        })
        .collect();
    out.extend(session.pending_decisions().iter().map(|o| Anomaly::UnusedChoice { option: o.clone() }));
    out
}

/// Every witness applied to `start` reaches exactly its outcome: an empty
/// queue, the outcome's live instances, timers and (traceable) anomalies.
pub fn check_witnesses(model: &Model, start: &PlaySession, exploration: &Exploration, context: &str) {
    let from = start.trace().steps.len();
    for (i, outcome) in exploration.outcomes.iter().enumerate() {
        let session = match replay_witness(start, model, &outcome.witness) {
            Ok(session) => session,
            Err(err) => panic!("{context}: outcome {i}'s witness does not replay: {err}\n{:?}", outcome.witness),
        };
        let halted = outcome.anomalies.contains(&Anomaly::StepLimit);
        if !halted {
            assert!(session.pending().is_empty(), "{context}: outcome {i}'s witness leaves items queued");
        }
        let live: BTreeMap<String, _> =
            session.instances().into_iter().map(|i| (i.name.clone(), (i.state, i.fields))).collect();
        for (name, value) in &live {
            assert_eq!(outcome.instances.get(name), Some(value), "{context}: outcome {i}, instance {name}");
        }
        assert!(outcome.instances.len() >= live.len(), "{context}: outcome {i} misses instances");
        let now = session.now();
        let mut timers: Vec<OutcomeTimer> = session
            .timers()
            .into_iter()
            .map(|t| OutcomeTimer {
                instance: t.instance,
                trigger: t.trigger,
                remaining: t.deadline.saturating_sub(now),
            })
            .collect();
        timers.sort();
        assert_eq!(timers, outcome.timers, "{context}: outcome {i}'s timers");
        let traceable: BTreeSet<Anomaly> = outcome
            .anomalies
            .iter()
            .filter(|a| !matches!(a, Anomaly::InputFailed { .. } | Anomaly::StepLimit))
            .cloned()
            .collect();
        assert_eq!(replayed_anomalies(model, &session, from), traceable, "{context}: outcome {i}'s anomalies");
    }
}

/// Outcomes are distinct.
pub fn check_distinct(exploration: &Exploration, context: &str) {
    assert_eq!(identities(exploration).len(), exploration.outcomes.len(), "{context}: duplicate outcomes");
}
