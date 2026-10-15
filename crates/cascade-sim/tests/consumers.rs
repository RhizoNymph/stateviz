//! Per-consumer delivery: `Step { consumer: Some(name) }` and the scenario's
//! `{ step: n, consumer: X }`.

mod common;
mod support;

use cascade_sim::{PlayAction, ScenarioErrorKind, SimError, scenario_to_yaml, simulate};
use common::{lines, model, scenario, strs};
use support::{RACE, add, apply, apply_err, fire, from_scenario, session, step, unnamed};

fn to(position: Option<u32>, consumer: &str) -> PlayAction {
    PlayAction::Step { choice: position, consumer: Some(consumer.to_owned()) }
}

const SUBSCRIBED: &str = r#"
machines:
  Order:
    initial: draft
    states: [draft, placed]
    transitions:
      - { from: draft, to: placed, on: place, emits: [OrderPlaced] }
  Audit:
    initial: idle
    states: [idle, logged]
    transitions:
      - { from: idle, to: logged, on: { event: OrderPlaced } }
  Receipt:
    initial: idle
    states: [idle, sent]
    transitions:
      - { from: idle, to: sent, on: send }
controllers:
  Mailer:
    on:
      OrderPlaced: { fire: Receipt.send, target: Receipt }
external:
  Customer: [Order.place]
"#;

fn placed(model: &cascade_core::Model) -> cascade_sim::PlaySession {
    session(
        model,
        &[
            add("o1", "Order", &[]),
            add("a1", "Audit", &[]),
            add("r1", "Receipt", &[]),
            fire("Customer", "Order.place", "o1"),
        ],
    )
}

#[test]
fn delivering_to_one_consumer_keeps_the_event_for_the_rest() {
    let model = model(SUBSCRIBED);
    let mut s = placed(&model);
    let id = s.pending()[0].id;
    apply(&mut s, &model, to(None, "Mailer"));
    let pending = s.pending();
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].id, id);
    assert_eq!(pending[0].consumers_left, strs(&["a1"]));
    // The fire Mailer queued sits behind the event.
    assert_eq!(pending[1].consumers_left, strs(&["r1"]));

    apply(&mut s, &model, to(Some(0), "a1"));
    assert!(!s.is_pending(id));
    assert_eq!(
        lines(&model, s.trace())[3..],
        strs(&["Mailer <- OrderPlaced", "Mailer fire r1 send", "a1 notify OrderPlaced", "a1 idle -> logged"])
    );
}

#[test]
fn the_head_is_recorded_as_position_zero() {
    let model = model(SUBSCRIBED);
    let mut s = placed(&model);
    apply(&mut s, &model, to(None, "Mailer"));
    assert_eq!(s.timeline().actions.last(), Some(&to(Some(0), "Mailer")));
}

#[test]
fn a_whole_step_after_a_partial_one_delivers_the_rest() {
    let model = model(SUBSCRIBED);
    let mut s = placed(&model);
    apply(&mut s, &model, to(None, "Mailer"));
    apply(&mut s, &model, step());
    assert_eq!(lines(&model, s.trace()).last().map(String::as_str), Some("a1 idle -> logged"));
    assert_eq!(s.pending().len(), 1);
}

#[test]
fn an_unknown_consumer_is_rejected_and_changes_nothing() {
    let model = model(SUBSCRIBED);
    let mut s = placed(&model);
    let before = (s.trace().clone(), s.timeline().clone());
    let err = apply_err(&mut s, &model, to(None, "Nobody"));
    match err {
        SimError::Action(ScenarioErrorKind::UnknownConsumer { consumer, consumers, .. }) => {
            assert_eq!(consumer, "Nobody");
            assert_eq!(consumers, strs(&["a1", "Mailer"]));
        }
        other => panic!("{other}"),
    }
    // A consumer that already had it is not a consumer any more.
    apply(&mut s, &model, to(None, "Mailer"));
    assert!(matches!(
        apply_err(&mut s, &model, to(None, "Mailer")),
        SimError::Action(ScenarioErrorKind::UnknownConsumer { .. })
    ));
    let _ = before;
}

#[test]
fn a_fire_takes_its_target_as_consumer() {
    let model = model(RACE);
    let mut s = session(
        &model,
        &[
            add("o1", "Order", &[("orderId", "1")]),
            add("s1", "Shipment", &[("orderId", "1")]),
            fire("PaymentGateway", "Order.capture_ok", "o1"),
            step(),
        ],
    );
    assert_eq!(s.pending()[0].consumers_left, strs(&["s1"]));
    assert!(matches!(
        apply_err(&mut s, &model, to(None, "o1")),
        SimError::Action(ScenarioErrorKind::UnknownConsumer { .. })
    ));
    let before = s.pending().len();
    apply(&mut s, &model, to(None, "s1"));
    assert_eq!(s.pending().len(), before - 1);
}

#[test]
fn delivering_to_the_second_controller_first_swaps_their_fires() {
    let model = model(RACE);
    let start = [
        add("o1", "Order", &[("orderId", "1")]),
        add("s1", "Shipment", &[("orderId", "1")]),
        fire("PaymentGateway", "Order.capture_ok", "o1"),
    ];
    let mut s = session(&model, &start);
    let consumers = s.pending()[0].consumers_left.clone();
    assert_eq!(consumers, strs(&["Fulfillment", "Billing"]));
    apply(&mut s, &model, to(None, "Billing"));
    apply(&mut s, &model, to(None, "Fulfillment"));
    let labels: Vec<String> = s.pending().iter().map(|p| p.label.clone()).collect();
    assert!(labels[0].starts_with("Billing"), "{labels:?}");
    assert!(labels[1].starts_with("Fulfillment"), "{labels:?}");
}

#[test]
fn per_consumer_steps_save_and_replay_exactly() {
    let model = model(SUBSCRIBED);
    let mut s = placed(&model);
    apply(&mut s, &model, to(None, "Mailer"));
    apply(&mut s, &model, to(None, "a1"));
    let saved = match s.to_scenario("per consumer") {
        Ok(saved) => saved,
        Err(err) => panic!("{err}"),
    };
    let text = scenario_to_yaml(&saved);
    assert!(text.contains("  - { step: 0, consumer: Mailer }\n"), "{text}");
    assert!(text.contains("  - { step: 0, consumer: a1 }\n"), "{text}");
    let again = scenario(&text);
    let replayed = from_scenario(&model, &again);
    assert_eq!(replayed.timeline().actions, s.timeline().actions);
    assert_eq!(unnamed(replayed.trace()), unnamed(s.trace()));
    match simulate(&model, &again) {
        Ok(trace) => assert_eq!(unnamed(&trace), unnamed(s.trace())),
        Err(err) => panic!("{err}"),
    }
}

#[test]
fn a_scenario_consumer_step_runs_in_batch() {
    let model = model(SUBSCRIBED);
    let text = "scenario: c\ninstances:\n  o1: Order\n  a1: Audit\n  r1: Receipt\nsteps:\n  - { source: Customer, fire: Order.place, target: o1 }\n  - { step: 0, consumer: Mailer }\nend: pause\n";
    let trace = common::run(&model, text);
    assert_eq!(lines(&model, &trace)[3..], strs(&["Mailer <- OrderPlaced", "Mailer fire r1 send"]));
    // An unknown consumer is reported at the directive.
    let err = common::run_err(&model, &text.replace("consumer: Mailer", "consumer: Nobody"));
    let SimError::Scenario(err) = err else { panic!("{err}") };
    assert!(matches!(err.diagnostics[0].kind, ScenarioErrorKind::UnknownConsumer { .. }));
    assert_eq!(err.diagnostics[0].span.start.line, 8);
}
