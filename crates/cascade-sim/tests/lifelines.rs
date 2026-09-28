//! Lifeline order follows definition order, so a scenario always lays out
//! the same way.

mod common;

use cascade_core::Model;
use cascade_sim::{Lifeline, TraceStepKind, simulate};
use common::{lifelines, model, run, scenario, strs};

/// Externals, machines and controllers are each declared in an order that
/// differs from the order the scenario uses them in.
const DEFINITION: &str = r#"
machines:
  Alpha:
    states: [a0, a1]
    transitions:
      - { from: a0, to: a1, on: go, emits: [AlphaDone] }
  Beta:
    states: [b0, b1]
    transitions:
      - { from: b0, to: b1, on: go, emits: [BetaDone] }
  Gamma:
    states: [g0, g1]
    transitions:
      - { from: g0, to: g1, on: go }
controllers:
  Unused:
    on:
      Nothing: { fire: Gamma.go }
  Second:
    on:
      BetaDone: { fire: Gamma.go, target: new Gamma }
  First:
    on:
      AlphaDone: { fire: Gamma.go, target: all Gamma }
external:
  Idle: [Alpha.go]
  Zed: [Beta.go]
  Ann: [Alpha.go]
"#;

const SCENARIO: &str = r#"scenario: layout
instances:
  b1: Beta
  g2: Gamma
  a1: Alpha
  g1: Gamma
steps:
  - { source: Zed, fire: Beta.go, target: b1 }
  - { source: Ann, fire: Alpha.go, target: a1 }
"#;

#[test]
fn lifelines_follow_definition_order() {
    let model = model(DEFINITION);
    let trace = run(&model, SCENARIO);
    assert_eq!(
        lifelines(&model, &trace),
        strs(&[
            // Sources in model order; `Idle` never fires, so it is left out.
            "ext:Zed",
            "ext:Ann",
            // Instances by machine in model order, then declaration order,
            // spawned instances last within their machine.
            "Alpha:a1",
            "Beta:b1",
            "Gamma:g2",
            "Gamma:g1",
            "Gamma:gamma1",
            // Controllers in model order; `Unused` never acts.
            "ctl:Second",
            "ctl:First",
        ])
    );
}

#[test]
fn every_instance_has_a_lifeline_and_a_final_state() {
    let model = model(DEFINITION);
    let trace = run(&model, "scenario: idle\ninstances:\n  g1: Gamma\n  a1: Alpha\nsteps: []\n");
    assert_eq!(lifelines(&model, &trace), strs(&["Alpha:a1", "Gamma:g1"]));
    assert_eq!(trace.final_states.len(), 2);
    assert!(trace.steps.is_empty());
}

#[test]
fn step_lifelines_point_at_the_right_participants() {
    let model = model(DEFINITION);
    let trace = run(&model, SCENARIO);
    for step in &trace.steps {
        let ok = match &step.kind {
            TraceStepKind::ExternalFire { source, target, .. } => {
                matches!(trace.lifelines[source.index()], Lifeline::External { .. })
                    && matches!(trace.lifelines[target.index()], Lifeline::Instance { .. })
            }
            TraceStepKind::Transition { instance, .. }
            | TraceStepKind::Dropped { instance, .. }
            | TraceStepKind::Emit { instance, .. } => {
                matches!(trace.lifelines[instance.index()], Lifeline::Instance { .. })
            }
            TraceStepKind::Deliver { controller, .. } | TraceStepKind::NoTarget { controller, .. } => {
                matches!(trace.lifelines[controller.index()], Lifeline::Controller { .. })
            }
            TraceStepKind::Fire { controller, target: instance, .. }
            | TraceStepKind::Spawn { controller, instance, .. } => {
                matches!(trace.lifelines[controller.index()], Lifeline::Controller { .. })
                    && matches!(trace.lifelines[instance.index()], Lifeline::Instance { .. })
            }
            TraceStepKind::Ambiguous { controller, candidates, .. } => {
                matches!(trace.lifelines[controller.index()], Lifeline::Controller { .. })
                    && candidates.iter().all(|c| matches!(trace.lifelines[c.index()], Lifeline::Instance { .. }))
            }
        };
        assert!(ok, "{step:?}");
    }
    for ix in trace.final_states.keys() {
        assert!(matches!(trace.lifelines[ix.index()], Lifeline::Instance { .. }));
    }
}

#[test]
fn the_same_scenario_always_lays_out_the_same_way() {
    let model: Model = model(DEFINITION);
    let first = simulate(&model, &scenario(SCENARIO));
    for _ in 0..20 {
        assert_eq!(simulate(&model, &scenario(SCENARIO)), first);
    }
    // Reloading the model does not change the trace either.
    let reloaded = common::model(DEFINITION);
    assert_eq!(simulate(&reloaded, &scenario(SCENARIO)), first);
}
