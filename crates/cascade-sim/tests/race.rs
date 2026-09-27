//! Race candidates replayed in both orders.

mod common;

use cascade_core::ids::RuleId;
use cascade_core::{FindingDetail, Model};
use cascade_sim::{SimError, TraceStepKind, race_orderings, race_runs, simulate};
use common::{finals, lines, model, scenario, strs};

const FIXTURE: &str = include_str!("fixtures/race/cascade.yaml");
const FIXTURE_SCENARIO: &str = include_str!("fixtures/race/scenario.yaml");

fn rule_of(model: &Model, controller: &str) -> RuleId {
    match model.rules().find(|(_, r)| model.controller(r.controller).name == controller) {
        Some((id, _)) => id,
        None => panic!("no rule in controller {controller}"),
    }
}

fn race(model: &Model, origin: &str, first: &str, second: &str) -> FindingDetail {
    let first = rule_of(model, first);
    let Some(origin) = model.event_by_name(origin) else { panic!("no event {origin}") };
    FindingDetail::RaceCandidate {
        origin,
        machine: model.trigger(model.rule(first).trigger).machine,
        first,
        second: rule_of(model, second),
    }
}

#[test]
fn both_orderings_of_the_fixture_race() {
    let model = model(FIXTURE);
    let finding = race(&model, "OrderPaid", "Fulfillment", "Billing");
    let traces = match race_orderings(&model, &scenario(FIXTURE_SCENARIO), &finding) {
        Ok(t) => t,
        Err(err) => panic!("{err}"),
    };

    assert_eq!(traces.as_queued.ordering.as_deref(), Some("Fulfillment first"));
    assert_eq!(
        lines(&model, &traces.as_queued),
        strs(&[
            "ext PaymentGateway -> o1 capture_ok",
            "o1 pending -> paid",
            "o1 emit OrderPaid",
            "Fulfillment <- OrderPaid",
            "Fulfillment fire s1 start",
            "Billing <- OrderPaid",
            "Billing fire s1 hold",
            "s1 idle -> picking",
            "s1 drop hold @ picking",
        ])
    );
    assert_eq!(finals(&model, &traces.as_queued), ["o1: paid", "s1: picking"]);

    assert_eq!(traces.swapped.ordering.as_deref(), Some("Billing first"));
    assert_eq!(
        lines(&model, &traces.swapped),
        strs(&[
            "ext PaymentGateway -> o1 capture_ok",
            "o1 pending -> paid",
            "o1 emit OrderPaid",
            "Fulfillment <- OrderPaid",
            "Fulfillment fire s1 start",
            "Billing <- OrderPaid",
            "Billing fire s1 hold",
            "s1 idle -> on_hold",
            "s1 drop start @ on_hold",
        ])
    );
    assert_eq!(finals(&model, &traces.swapped), ["o1: paid", "s1: on_hold"]);
    // Each delivery still points at the fire it delivers.
    assert_eq!(traces.swapped.steps[7].cause.map(|c| c.0), Some(6));
    assert_eq!(traces.swapped.steps[8].cause.map(|c| c.0), Some(4));
    // Both orderings share lifelines, so they can be drawn side by side.
    assert_eq!(traces.as_queued.lifelines, traces.swapped.lifelines);
}

#[test]
fn the_as_queued_ordering_is_the_plain_run() {
    let model = model(FIXTURE);
    let s = scenario(FIXTURE_SCENARIO);
    let finding = race(&model, "OrderPaid", "Fulfillment", "Billing");
    let (Ok(traces), Ok(mut plain)) = (race_orderings(&model, &s, &finding), simulate(&model, &s)) else {
        panic!("expected both runs to succeed");
    };
    plain.ordering = Some("Fulfillment first".to_owned());
    assert_eq!(traces.as_queued, plain);
}

#[test]
fn the_rule_order_in_the_finding_does_not_matter() {
    let model = model(FIXTURE);
    let s = scenario(FIXTURE_SCENARIO);
    let a = race_orderings(&model, &s, &race(&model, "OrderPaid", "Fulfillment", "Billing"));
    let b = race_orderings(&model, &s, &race(&model, "OrderPaid", "Billing", "Fulfillment"));
    assert!(a.is_ok());
    assert_eq!(a, b);
}

#[test]
fn race_runs_keep_payloads() {
    let model = model(FIXTURE);
    let finding = race(&model, "OrderPaid", "Fulfillment", "Billing");
    let runs = match race_runs(&model, &scenario(FIXTURE_SCENARIO), &finding) {
        Ok(r) => r,
        Err(err) => panic!("{err}"),
    };
    let payload = |run: &cascade_sim::SimRun| run.payloads.get(&cascade_sim::StepIx(2)).cloned();
    assert_eq!(payload(&runs.as_queued), payload(&runs.swapped));
    assert_eq!(payload(&runs.as_queued).and_then(|p| p.get("orderId").cloned()).as_deref(), Some("1"));
    assert_eq!(runs.swapped.trace.ordering.as_deref(), Some("Billing first"));
}

#[test]
fn other_findings_are_not_races() {
    let model = model(FIXTURE);
    let Some(event) = model.event_by_name("OrderPaid") else { panic!() };
    let err = race_orderings(&model, &scenario(FIXTURE_SCENARIO), &FindingDetail::UnhandledEvent { event });
    assert_eq!(err, Err(SimError::NotARace));
}

#[test]
fn a_scenario_that_never_reaches_the_race() {
    let model = model(FIXTURE);
    let finding = race(&model, "OrderPaid", "Fulfillment", "Billing");

    // Nothing is paid.
    let idle = "scenario: idle\ninstances:\n  s1: { machine: Shipment, fields: { orderId: 1 } }\nsteps: []\n";
    assert!(matches!(
        race_orderings(&model, &scenario(idle), &finding),
        Err(SimError::RaceNotReached { origin, .. }) if origin == "OrderPaid"
    ));

    // Paid, but there is no shipment to fire at.
    let no_shipment = "scenario: no shipment\ninstances:\n  o1: { machine: Order, fields: { orderId: 1 } }\nsteps:\n  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }\n";
    let err = race_orderings(&model, &scenario(no_shipment), &finding);
    assert!(matches!(err, Err(SimError::RaceNotReached { .. })), "{err:?}");
    if let Err(err) = err {
        assert!(err.to_string().contains("never reaches"), "{err}");
    }
}

#[test]
fn invalid_scenarios_are_reported_before_racing() {
    let model = model(FIXTURE);
    let finding = race(&model, "OrderPaid", "Fulfillment", "Billing");
    let bad = "scenario: bad\nsteps:\n  - { source: Nobody, fire: Order.capture_ok }\n";
    assert!(matches!(race_orderings(&model, &scenario(bad), &finding), Err(SimError::Scenario(_))));
}

/// Compliance only fires `hold` after Billing's invoice settles, so FIFO
/// delivers Fulfillment's `start` long before `hold` is even queued.
const LATE: &str = r#"
machines:
  Order:
    initial: pending
    fields: [orderId]
    states: [pending, paid]
    transitions:
      - { from: pending, to: paid, on: capture_ok, emits: [OrderPaid] }
  Invoice:
    initial: open
    fields: [orderId]
    states: [open, settled]
    transitions:
      - { from: open, to: settled, on: settle, emits: [InvoiceSettled] }
  Shipment:
    initial: idle
    fields: [orderId]
    states: [idle, picking, on_hold]
    transitions:
      - { from: idle, to: picking, on: start }
      - { from: idle, to: on_hold, on: hold }
controllers:
  Fulfillment:
    on:
      OrderPaid:
        - fire: Shipment.start
          target: Shipment where orderId == event.orderId
  Billing:
    on:
      OrderPaid:
        - fire: Invoice.settle
          target: Invoice where orderId == event.orderId
  Compliance:
    on:
      InvoiceSettled:
        - fire: Shipment.hold
          target: Shipment where orderId == event.orderId
external:
  PaymentGateway: [Order.capture_ok]
"#;

const LATE_SCENARIO: &str = r#"scenario: late hold
instances:
  o1: { machine: Order, fields: { orderId: 1 } }
  i1: { machine: Invoice, fields: { orderId: 1 } }
  s1: { machine: Shipment, fields: { orderId: 1 } }
steps:
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
"#;

#[test]
fn a_fire_that_is_not_yet_queued_can_still_go_first() {
    let model = model(LATE);
    let finding = race(&model, "OrderPaid", "Fulfillment", "Compliance");
    let traces = match race_orderings(&model, &scenario(LATE_SCENARIO), &finding) {
        Ok(t) => t,
        Err(err) => panic!("{err}"),
    };
    assert_eq!(traces.as_queued.ordering.as_deref(), Some("Fulfillment first"));
    assert_eq!(
        lines(&model, &traces.as_queued)[7..],
        strs(&[
            "s1 idle -> picking",
            "i1 open -> settled",
            "i1 emit InvoiceSettled",
            "Compliance <- InvoiceSettled",
            "Compliance fire s1 hold",
            "s1 drop hold @ picking",
        ])
    );
    assert_eq!(traces.swapped.ordering.as_deref(), Some("Compliance first"));
    assert_eq!(
        lines(&model, &traces.swapped)[7..],
        strs(&[
            "i1 open -> settled",
            "i1 emit InvoiceSettled",
            "Compliance <- InvoiceSettled",
            "Compliance fire s1 hold",
            "s1 idle -> on_hold",
            "s1 drop start @ on_hold",
        ])
    );
    assert!(matches!(traces.swapped.steps[12].kind, TraceStepKind::Dropped { .. }));
    assert_eq!(traces.swapped.steps[12].cause.map(|c| c.0), Some(4));
}

#[test]
fn a_fire_caused_by_the_other_cannot_be_swapped() {
    // `hold` is only fired because `start` was delivered.
    let model = model(
        r#"
machines:
  Order:
    initial: pending
    fields: [orderId]
    states: [pending, paid]
    transitions:
      - { from: pending, to: paid, on: capture_ok, emits: [OrderPaid] }
  Shipment:
    initial: idle
    fields: [orderId]
    states: [idle, picking, on_hold]
    transitions:
      - { from: idle, to: picking, on: start, emits: [ShipmentStarted] }
      - { from: picking, to: on_hold, on: hold }
controllers:
  Fulfillment:
    on:
      OrderPaid:
        - fire: Shipment.start
          target: Shipment where orderId == event.orderId
  Compliance:
    on:
      ShipmentStarted:
        - fire: Shipment.hold
          target: Shipment where orderId == event.orderId
external:
  PaymentGateway: [Order.capture_ok]
"#,
    );
    let finding = race(&model, "OrderPaid", "Fulfillment", "Compliance");
    let err = race_orderings(&model, &scenario(FIXTURE_SCENARIO), &finding);
    assert!(matches!(err, Err(SimError::RaceNotSwappable { .. })), "{err:?}");
}
