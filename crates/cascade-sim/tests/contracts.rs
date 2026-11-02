//! The contract surface for timers, choices and per-consumer delivery: the
//! new directives parse, validate and are written by `scenario_to_yaml`;
//! the session queries answer trivially for a model without timers or
//! choices; `consumers_left` and the state key are real. (The semantics
//! have their own test files: `timers.rs`, `choices.rs`, `consumers.rs`.)

mod common;
mod support;

use std::collections::BTreeMap;

use cascade_core::Model;
use cascade_core::expr::Duration;
use cascade_sim::{
    Directive, LifelineIx, PlayAction, PlaySession, ScenarioEntry, ScenarioErrorKind, SimError, StepIx, StepTiming,
    Trace, TraceStep, TraceStepKind, parse_scenario, scenario_to_yaml, step_text, validate,
};
use common::{has, model, scenario, strs};
use support::{RACE, add, apply_err, fire, session, step};

fn race_session(model: &Model, more: &[PlayAction]) -> PlaySession {
    let mut actions = vec![add("o1", "Order", &[("orderId", "1")]), add("s1", "Shipment", &[("orderId", "1")])];
    actions.extend_from_slice(more);
    session(model, &actions)
}

fn capture() -> PlayAction {
    fire("PaymentGateway", "Order.capture_ok", "o1")
}

const NEW_DIRECTIVES: &str = r#"scenario: new directives
instances:
  o1: { machine: Order, fields: { orderId: "1" } }
  s1: { machine: Shipment, fields: { orderId: "1" } }
steps:
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
  - wait
  - { wait: 30s }
  - { wait: 2m, timing: immediate }
  - { choose: "card declined" }
  - { step: 1, consumer: Fulfillment }
end: pause
"#;

#[test]
fn new_directives_parse() {
    let s = scenario(NEW_DIRECTIVES);
    let directives: Vec<&Directive> = s
        .entries()
        .filter_map(|e| match e {
            ScenarioEntry::Directive(d) => Some(d),
            ScenarioEntry::Fire(_) => None,
        })
        .collect();
    assert_eq!(directives.len(), 5);
    assert!(
        matches!(directives[0], Directive::Wait { by: None, timing: StepTiming::AfterQuiescence, .. }),
        "{:?}",
        directives[0]
    );
    match directives[1] {
        Directive::Wait { by: Some(by), timing: StepTiming::AfterQuiescence, .. } => {
            assert_eq!(by.value, Duration::from_secs(30));
        }
        other => panic!("{other:?}"),
    }
    match directives[2] {
        Directive::Wait { by: Some(by), timing: StepTiming::Immediate, .. } => {
            assert_eq!(by.value, Duration::from_secs(120));
        }
        other => panic!("{other:?}"),
    }
    match directives[3] {
        Directive::Choose { option, .. } => assert_eq!(option.value, "card declined"),
        other => panic!("{other:?}"),
    }
    match directives[4] {
        Directive::Deliver { choice: Some(1), consumer: Some(consumer), .. } => {
            assert_eq!(consumer.value, "Fulfillment");
        }
        other => panic!("{other:?}"),
    }
    // Every directive keeps its line.
    let lines: Vec<u32> = directives.iter().map(|d| d.span().start.line).collect();
    assert_eq!(lines, [7, 8, 9, 10, 11]);
}

#[test]
fn new_directives_round_trip_through_yaml() {
    let s = scenario(NEW_DIRECTIVES);
    let text = scenario_to_yaml(&s);
    assert!(text.contains("  - wait\n"), "{text}");
    assert!(text.contains("  - { wait: 30s }\n"), "{text}");
    assert!(text.contains("  - { wait: 2m, timing: immediate }\n"), "{text}");
    assert!(text.contains("  - { choose: \"card declined\" }\n"), "{text}");
    assert!(text.contains("  - { step: 1, consumer: Fulfillment }\n"), "{text}");
    let again = scenario(&text);
    assert_eq!(scenario_to_yaml(&again), text);
}

#[test]
fn a_bare_wait_with_a_timing_writes_the_mapping_form() {
    let s = scenario("scenario: w\nsteps:\n  - { wait: ~, timing: immediate }\n");
    match s.trailing.as_slice() {
        [Directive::Wait { by: None, timing: StepTiming::Immediate, .. }] => {}
        other => panic!("{other:?}"),
    }
    let text = scenario_to_yaml(&s);
    assert!(text.contains("  - { wait: ~, timing: immediate }\n"), "{text}");
    assert_eq!(scenario_to_yaml(&scenario(&text)), text);
}

#[test]
fn malformed_new_directives_are_reported() {
    let err = parse_scenario("scenario: a\nsteps:\n  - { wait: soon }\n").unwrap_err();
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::InvalidDuration { text, .. } if text == "soon")), "{err}");
    assert_eq!(err.diagnostics[0].span.start.line, 3);

    let err = parse_scenario("scenario: a\nsteps:\n  - { wait: 1s, timing: later }\n").unwrap_err();
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownTiming { .. })), "{err}");

    let err = parse_scenario("scenario: a\nsteps:\n  - { choose: x, target: o1 }\n").unwrap_err();
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownKey { key, .. } if key == "target")), "{err}");

    let err = parse_scenario("scenario: a\nsteps:\n  - { step: 0, consumer: \"not a name\" }\n").unwrap_err();
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::InvalidName { .. })), "{err}");
}

#[test]
fn new_directives_validate_against_the_model() {
    let model = model(RACE);
    if let Err(err) = validate(&model, &scenario(NEW_DIRECTIVES)) {
        panic!("{err}");
    }
}

#[test]
fn a_wait_with_nothing_armed_is_rejected_and_changes_nothing() {
    let model = model(RACE);
    let mut session = race_session(&model, &[capture()]);
    let before = session.trace().clone();
    let actions = session.timeline().actions.clone();
    let err = apply_err(&mut session, &model, PlayAction::Advance { by: None });
    assert!(matches!(err, SimError::Action(ScenarioErrorKind::NoTimerArmed)), "{err}");
    assert_eq!(session.trace(), &before);
    assert_eq!(session.timeline().actions, actions);
}

#[test]
fn consumers_left_lists_handlers_for_events_and_the_target_for_fires() {
    let model = model(RACE);
    let session = race_session(&model, &[capture()]);
    let pending = session.pending();
    let consumers: Vec<Vec<String>> = pending.iter().map(|p| p.consumers_left.clone()).collect();
    assert_eq!(consumers, [strs(&["Fulfillment", "Billing"])]);

    let session = race_session(&model, &[capture(), step()]);
    let consumers: Vec<Vec<String>> = session.pending().iter().map(|p| p.consumers_left.clone()).collect();
    assert_eq!(consumers, [strs(&["s1"]), strs(&["s1"])]);
}

#[test]
fn clock_timers_and_choices_are_empty_without_timers_or_choices() {
    let model = model(RACE);
    let session = race_session(&model, &[capture()]);
    assert_eq!(session.now(), Duration::ZERO);
    assert!(session.timers().is_empty());
    assert!(session.pending_decisions().is_empty());
    assert_eq!(session.choice_points(&model, None, None), Ok(Vec::new()));
}

#[test]
fn state_keys_are_equal_for_equal_states() {
    let model = model(RACE);
    let a = race_session(&model, &[capture()]);
    let b = race_session(&model, &[capture()]);
    assert_eq!(a.state_key(), b.state_key());

    // Different queue contents and different leaves give different keys.
    let stepped = race_session(&model, &[capture(), step()]);
    assert_ne!(a.state_key(), stepped.state_key());
    let start = race_session(&model, &[]);
    assert_ne!(a.state_key(), start.state_key());

    // Delivering the two contested fires in either order ends in different
    // states; delivering them in the same order ends in the same one.
    let first = race_session(&model, &[capture(), step(), step(), step()]);
    let again = race_session(&model, &[capture(), PlayAction::RunUntilQuiet]);
    assert_eq!(first.state_key(), again.state_key());
    let swapped = race_session(&model, &[capture(), step(), support::choose(1), step()]);
    assert_ne!(first.state_key(), swapped.state_key());
}

#[test]
fn state_keys_ignore_ids_and_the_trace() {
    let model = model(RACE);
    // One session reaches the start state after a removal: the removed name
    // is part of the key, the trace is not.
    let mut with_removed = race_session(&model, &[]);
    support::apply(&mut with_removed, &model, add("x1", "Order", &[]));
    support::apply(&mut with_removed, &model, support::remove("x1"));
    let plain = race_session(&model, &[]);
    assert_ne!(with_removed.state_key(), plain.state_key());

    // A quiet run adds a timeline action but changes no state.
    let quiet = race_session(&model, &[PlayAction::RunUntilQuiet]);
    assert_eq!(quiet.state_key(), plain.state_key());
}

#[test]
fn new_step_kinds_have_text() {
    let model = model(RACE);
    let session = race_session(&model, &[capture()]);
    let base = session.trace();
    let order = model.machine_by_name("Order").unwrap();
    let capture_ok = model.trigger_by_name(order, "capture_ok").unwrap();
    let paid = model.event_by_name("OrderPaid").unwrap();
    let o1 = base
        .lifelines
        .iter()
        .position(|l| matches!(l, cascade_sim::Lifeline::Instance { name, .. } if name == "o1"))
        .map(|i| LifelineIx(u32::try_from(i).unwrap()))
        .unwrap();
    let trace = Trace {
        steps: vec![
            TraceStep {
                cause: None,
                kind: TraceStepKind::Wait { by: Duration::from_secs(30), now: Duration::from_secs(90) },
            },
            TraceStep {
                cause: Some(StepIx(0)),
                kind: TraceStepKind::TimerExpired { instance: o1, trigger: capture_ok },
            },
            TraceStep { cause: None, kind: TraceStepKind::Notify { instance: o1, event: paid } },
            TraceStep {
                cause: None,
                kind: TraceStepKind::Choice {
                    at: o1,
                    options: strs(&["card declined", "else"]),
                    chosen: "card declined".to_owned(),
                    defaulted: false,
                },
            },
            TraceStep {
                cause: None,
                kind: TraceStepKind::Choice {
                    at: o1,
                    options: strs(&["card declined", "else"]),
                    chosen: "card declined".to_owned(),
                    defaulted: true,
                },
            },
            TraceStep {
                cause: None,
                kind: TraceStepKind::SetFields {
                    instance: o1,
                    fields: BTreeMap::from([("orderId".to_owned(), "2".to_owned())]),
                },
            },
        ],
        ..base.clone()
    };
    let texts: Vec<String> = trace.steps.iter().map(|s| step_text(&model, &trace, s, None)).collect();
    assert_eq!(
        texts,
        [
            "waited 30s (now 90s)",
            "Order o1: timer capture_ok expired",
            "o1 receives OrderPaid",
            "o1 chose card declined",
            "o1 chose card declined (default)",
            "o1 sets {orderId: 2}",
        ]
    );
}
