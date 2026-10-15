//! Race replays still find and swap the contested fires when choice points
//! sit on the way (a `?choice` condition before a fire, choice guards at the
//! target).

mod common;

use cascade_core::ids::RuleId;
use cascade_core::{FindingDetail, Model};
use cascade_sim::race_orderings;
use common::{finals, model, scenario};

const CHOICES: &str = r#"
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
    states: [idle, picking, packing, on_hold]
    transitions:
      - { from: idle, to: picking, on: start, guard: "?by hand" }
      - { from: idle, to: packing, on: start, guard: "?by robot" }
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
        - fire: Shipment.hold
          target: Shipment where orderId == event.orderId
          when: "?risky"
external:
  PaymentGateway: [Order.capture_ok]
"#;

const SCENARIO: &str = r#"scenario: payment captured
instances:
  o1: { machine: Order, fields: { orderId: "1" } }
  s1: { machine: Shipment, fields: { orderId: "1" } }
steps:
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
"#;

fn rule_of(model: &Model, controller: &str) -> RuleId {
    match model.rules().find(|(_, r)| model.controller(r.controller).name == controller) {
        Some((id, _)) => id,
        None => panic!("no rule in controller {controller}"),
    }
}

#[test]
fn contested_fires_behind_choice_points_still_swap() {
    let model = model(CHOICES);
    let first = rule_of(&model, "Fulfillment");
    let Some(origin) = model.event_by_name("OrderPaid") else { panic!("no OrderPaid") };
    let finding = FindingDetail::RaceCandidate {
        origin,
        machine: model.trigger(model.rule(first).trigger).machine,
        first,
        second: rule_of(&model, "Billing"),
    };
    let traces = match race_orderings(&model, &scenario(SCENARIO), &finding) {
        Ok(traces) => traces,
        Err(err) => panic!("{err}"),
    };
    assert_eq!(finals(&model, &traces.as_queued), ["o1: paid", "s1: picking"]);
    assert_eq!(finals(&model, &traces.swapped), ["o1: paid", "s1: on_hold"]);
}
