//! Scene building stays interactive at the spec's scale: about 20
//! machines, 200 states and 50 controllers, each view in under a second
//! even in a debug build.

mod common;

use std::collections::BTreeMap;
use std::fmt::Write;
use std::time::{Duration, Instant};

use cascade_core::{Direction, ElementRef};
use cascade_scene::{ConeFocus, OutsideFocus, SceneBuilder, ViewKind, ViewState};
use cascade_sim::{Lifeline, LifelineIx, StepIx, Trace, TraceStep, TraceStepKind};
use common::*;

const MACHINES: usize = 20;
const STATES: usize = 10;
const CONTROLLERS: usize = 50;
const LIMIT: Duration = Duration::from_secs(1);

/// 20 machines of 10 states (plus a nested pair in every fifth machine), 15
/// transitions each, 150 events, 50 controllers handling three events
/// each, 5 external sources.
fn large_model() -> String {
    let mut y = String::from("machines:\n");
    for m in 0..MACHINES {
        let _ = writeln!(y, "  M{m}:\n    domain: d{}\n    states:", m % 4);
        for s in 0..STATES {
            if m % 5 == 0 && s == 5 {
                let _ = writeln!(y, "      - s5: {{ states: [inner0, inner1] }}");
            } else {
                let _ = writeln!(y, "      - s{s}");
            }
        }
        let _ = writeln!(y, "    transitions:");
        for s in 0..STATES {
            let emits = if s % 2 == 0 { format!(", emits: [E{m}_{s}]") } else { String::new() };
            let _ = writeln!(y, "      - {{ from: s{s}, to: s{}, on: t{s}{emits} }}", (s + 1) % STATES);
        }
        for s in 0..5 {
            let _ = writeln!(y, "      - {{ from: s{s}, to: s{}, on: jump{s}, guard: \"x > {s}\" }}", (s + 3) % STATES);
        }
        if m % 5 == 0 {
            let _ = writeln!(y, "      - {{ from: s5.inner0, to: s5.inner1, on: step }}");
        }
    }
    let _ = writeln!(y, "controllers:");
    for c in 0..CONTROLLERS {
        let _ = writeln!(y, "  C{c}:\n    on:");
        for k in 0..3 {
            let source = (c * 3 + k) % MACHINES;
            let event = (c + k * 2) % 5 * 2;
            let target = (source + 1 + c % 7) % MACHINES;
            let _ = writeln!(y, "      E{source}_{event}: [{{ fire: M{target}.t{} }}]", (c + k) % STATES);
        }
    }
    let _ = writeln!(y, "external:");
    for x in 0..5 {
        let triggers: Vec<String> = (0..4).map(|i| format!("M{}.t0", (x * 4 + i) % MACHINES)).collect();
        let _ = writeln!(y, "  X{x}: [{}]", triggers.join(", "));
    }
    y
}

fn timed(fx: &Fixture, view: &ViewState) -> (Duration, usize) {
    let mut builder = SceneBuilder::new();
    let start = Instant::now();
    let scene = fx.build(&mut builder, view);
    (start.elapsed(), scene.nodes.len())
}

/// A long trace: every machine's instance steps through its states.
fn long_trace(fx: &Fixture) -> Trace {
    let model = &fx.model;
    let mut lifelines: Vec<Lifeline> = model.external_ids().map(|source| Lifeline::External { source }).collect();
    let first_instance = lifelines.len();
    lifelines
        .extend(model.machines().map(|(machine, m)| Lifeline::Instance { machine, name: format!("{}-1", m.name) }));
    let first_controller = lifelines.len();
    lifelines.extend(model.controller_ids().map(|controller| Lifeline::Controller { controller }));
    let ix = |i: usize| LifelineIx(u32::try_from(i).expect("small"));
    let mut steps = Vec::new();
    for (i, t) in model.transition_ids().enumerate().take(250) {
        let tr = model.transition(t);
        let instance = ix(first_instance + tr.machine.index());
        let cause = steps.len().checked_sub(1).map(|c| StepIx(u32::try_from(c).expect("small")));
        steps.push(TraceStep {
            cause,
            kind: TraceStepKind::Transition { instance, transition: t, from: tr.from, to: tr.to },
        });
        if let Some(&event) = tr.emits.first() {
            steps.push(TraceStep { cause: None, kind: TraceStepKind::Emit { instance, event } });
            if let Some(&handler) = model.event(event).handlers.first() {
                let controller = ix(first_controller + model.handler(handler).controller.index());
                let emit = StepIx(u32::try_from(steps.len() - 1).expect("small"));
                steps
                    .push(TraceStep { cause: Some(emit), kind: TraceStepKind::Deliver { controller, event, handler } });
                if let Some(&rule) = model.handler(handler).rules.first() {
                    let target = ix(first_instance + model.trigger(model.rule(rule).trigger).machine.index());
                    steps.push(TraceStep { cause: None, kind: TraceStepKind::Fire { controller, target, rule } });
                }
            }
        }
        let _ = i;
    }
    Trace { scenario: "long".into(), ordering: None, lifelines, steps, final_states: BTreeMap::new() }
}

#[test]
fn every_view_builds_within_a_second_at_spec_scale() {
    let mut fx = Fixture::new(&large_model());
    assert_eq!(fx.model.machine_count(), MACHINES);
    assert!(fx.model.state_count() >= 200, "{} states", fx.model.state_count());
    assert_eq!(fx.model.controller_count(), CONTROLLERS);
    assert!(fx.model.transition_count() >= 300);
    fx.traces = vec![long_trace(&fx)];

    let pick = fx.model.transition_ids().nth(17).map(|t| fx.model.key_of(ElementRef::Transition(t))).expect("t");
    let views = [
        ("causal", ViewState::default()),
        (
            "causal cone",
            ViewState {
                selection: vec![pick.clone()],
                cone: Some(ConeFocus { direction: Direction::Forward, depth: None }),
                ..ViewState::default()
            },
        ),
        (
            "causal hide",
            ViewState {
                selection: vec![pick.clone()],
                cone: Some(ConeFocus { direction: Direction::Backward, depth: Some(2) }),
                outside: OutsideFocus::Hide,
                ..ViewState::default()
            },
        ),
        (
            "causal stubs",
            ViewState { hidden_machines: (0..10).map(|m| format!("M{m}")).collect(), ..ViewState::default() },
        ),
        ("structure", ViewState { view: ViewKind::Structure, ..ViewState::default() }),
        (
            "structure collapsed",
            ViewState {
                view: ViewKind::Structure,
                collapsed: [k("machine:M3"), k("state:M0:s5")].into_iter().collect(),
                search: Some("s4".into()),
                ..ViewState::default()
            },
        ),
        ("trace", ViewState { view: ViewKind::Trace, selection: vec![pick], ..ViewState::default() }),
        ("matrix", ViewState { view: ViewKind::Matrix, ..ViewState::default() }),
    ];
    for (name, view) in &views {
        let (elapsed, nodes) = timed(&fx, view);
        assert!(nodes > 0, "{name} drew nothing");
        assert!(elapsed < LIMIT, "{name} took {elapsed:?} for {nodes} nodes");
    }
}

#[test]
fn rebuilding_with_new_emphasis_is_cheap() {
    let fx = Fixture::new(&large_model());
    let mut builder = SceneBuilder::new();
    let _ = fx.build(&mut builder, &ViewState::default());
    let start = Instant::now();
    for t in fx.model.transition_ids().take(10) {
        let key = fx.model.key_of(ElementRef::Transition(t));
        let view = ViewState {
            selection: vec![key],
            cone: Some(ConeFocus { direction: Direction::Forward, depth: Some(3) }),
            ..ViewState::default()
        };
        let _ = fx.build(&mut builder, &view);
    }
    assert_eq!(builder.layouts_run(), 1, "no relayout for emphasis");
    assert!(start.elapsed() < LIMIT * 2, "ten emphasis rebuilds took {:?}", start.elapsed());
}
