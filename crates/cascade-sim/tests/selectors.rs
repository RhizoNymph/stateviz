//! Target selectors: one, all, spawn, no target, ambiguous.

mod common;

use cascade_core::Model;
use cascade_sim::TraceStepKind;
use common::{finals, lifelines, lines, model, run, strs};

const DEFINITION: &str = r#"
machines:
  Order:
    initial: pending
    fields: [orderId]
    states: [pending, paid]
    transitions:
      - { from: pending, to: paid, on: capture_ok, emits: [OrderPaid] }
  Shipment:
    initial: idle
    fields: [orderId, carrier]
    states: [idle, picking, notified, shipped]
    transitions:
      - { from: idle, to: picking, on: start }
      - { from: idle, to: notified, on: notify }
      - { from: picking, to: shipped, on: handoff }
  Log:
    states: [ready]
    transitions:
      - { from: ready, to: ready, on: record }
controllers:
  Fulfillment:
    on:
      OrderPaid:
        - fire: Shipment.start
          target: Shipment where orderId == event.orderId
  Broadcast:
    on:
      OrderPaid:
        - fire: Shipment.notify
          target: all Shipment where orderId == event.orderId and carrier == ups
  Audit:
    on:
      OrderPaid: { fire: Log.record }
external:
  PaymentGateway: [Order.capture_ok]
  Warehouse: [Shipment.handoff]
"#;

fn paid(model: &Model, instances: &str) -> Vec<String> {
    let text = format!(
        "scenario: s\ninstances:\n  o1: {{ machine: Order, fields: {{ orderId: 1 }} }}\n{instances}steps:\n  - {{ source: PaymentGateway, fire: Order.capture_ok, target: o1 }}\n"
    );
    lines(model, &run(model, &text))
}

#[test]
fn one_all_and_singleton_selectors() {
    let model = model(DEFINITION);
    let got = paid(
        &model,
        "  s1: { machine: Shipment, fields: { orderId: 1, carrier: ups } }\n  s2: { machine: Shipment, fields: { orderId: 2, carrier: ups } }\n  s3: { machine: Shipment, fields: { orderId: 1, carrier: dhl } }\n  log: Log\n",
    );
    assert_eq!(
        got,
        strs(&[
            "ext PaymentGateway -> o1 capture_ok",
            "o1 pending -> paid",
            "o1 emit OrderPaid",
            "Fulfillment <- OrderPaid",
            "Fulfillment ambiguous start [s1, s3]",
            "Broadcast <- OrderPaid",
            "Broadcast fire s1 notify",
            "Audit <- OrderPaid",
            "Audit fire log record",
            "s1 idle -> notified",
            "log ready -> ready",
        ])
    );
}

#[test]
fn fan_out_fires_at_every_match_in_instance_order() {
    let model = model(DEFINITION);
    let got = paid(
        &model,
        "  s2: { machine: Shipment, fields: { orderId: 1, carrier: ups } }\n  s1: { machine: Shipment, fields: { orderId: 1, carrier: ups } }\n  log: Log\n",
    );
    assert_eq!(
        got[5..9],
        strs(&["Broadcast <- OrderPaid", "Broadcast fire s2 notify", "Broadcast fire s1 notify", "Audit <- OrderPaid"])
    );
}

#[test]
fn selectors_that_match_nothing_record_no_target() {
    let model = model(DEFINITION);
    let got = paid(&model, "  s9: { machine: Shipment, fields: { orderId: 9, carrier: ups } }\n  log: Log\n");
    assert_eq!(
        got[3..7],
        strs(&[
            "Fulfillment <- OrderPaid",
            "Fulfillment no-target start",
            "Broadcast <- OrderPaid",
            "Broadcast no-target notify",
        ])
    );
}

#[test]
fn a_missing_field_or_payload_key_never_matches() {
    let model = model(DEFINITION);
    // s1 has no orderId at all.
    let got = paid(&model, "  s1: { machine: Shipment, fields: { carrier: ups } }\n  log: Log\n");
    assert_eq!(got[4], "Fulfillment no-target start");
}

#[test]
fn a_singleton_selector_with_several_instances_is_ambiguous() {
    let model = model(DEFINITION);
    let got = paid(&model, "  a: Log\n  b: Log\n");
    let audit: Vec<&String> = got.iter().filter(|l| l.starts_with("Audit")).collect();
    assert_eq!(audit, [&"Audit <- OrderPaid".to_owned(), &"Audit ambiguous record [a, b]".to_owned()]);
}

const SPAWNING: &str = r#"
machines:
  Order:
    initial: pending
    fields: [orderId]
    states: [pending, paid]
    transitions:
      - { from: pending, to: paid, on: capture_ok, emits: [OrderPaid] }
  Shipment:
    initial: idle
    fields: [orderId, carrier, note]
    states: [idle, picking, shipped]
    transitions:
      - { from: idle, to: picking, on: start }
      - { from: picking, to: shipped, on: handoff }
controllers:
  Fulfillment:
    on:
      OrderPaid:
        - fire: Shipment.start
          target: new Shipment with orderId = event.orderId, carrier = ups, note = event.note
  Tracker:
    on:
      OrderPaid:
        - fire: Shipment.handoff
          target: Shipment where orderId == event.orderId and carrier == ups
external:
  PaymentGateway: [Order.capture_ok]
  Warehouse: [Shipment.handoff]
"#;

#[test]
fn spawn_creates_an_instance_and_fires_at_it() {
    let model = model(SPAWNING);
    let trace = run(
        &model,
        r#"scenario: spawn
instances:
  o1: { machine: Order, fields: { orderId: 1 } }
  shipment1: { machine: Shipment, fields: { orderId: 7 }, state: shipped }
steps:
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
  - { source: Warehouse, fire: Shipment.handoff, target: shipment2 }
"#,
    );
    assert_eq!(
        lines(&model, &trace),
        strs(&[
            "ext PaymentGateway -> o1 capture_ok",
            "o1 pending -> paid",
            "o1 emit OrderPaid",
            "Fulfillment <- OrderPaid",
            // `shipment1` is taken, so the spawned instance is `shipment2`.
            "Fulfillment spawn shipment2",
            "Fulfillment fire shipment2 start",
            // Later rules see the spawned instance and its assigned fields.
            "Tracker <- OrderPaid",
            "Tracker fire shipment2 handoff",
            "shipment2 idle -> picking",
            "shipment2 picking -> shipped",
            // Scenario steps can target spawned instances by name.
            "ext Warehouse -> shipment2 handoff",
            "shipment2 drop handoff @ shipped",
        ])
    );
    assert_eq!(trace.steps[5].cause, Some(cascade_sim::StepIx(4)));
    assert!(matches!(trace.steps[4].kind, TraceStepKind::Spawn { .. }));
    // Spawned lifelines follow the declared instances of their machine.
    assert_eq!(
        lifelines(&model, &trace),
        strs(&[
            "ext:PaymentGateway",
            "ext:Warehouse",
            "Order:o1",
            "Shipment:shipment1",
            "Shipment:shipment2",
            "ctl:Fulfillment",
            "ctl:Tracker"
        ])
    );
    assert_eq!(finals(&model, &trace), ["o1: paid", "shipment1: shipped", "shipment2: shipped"]);
}

#[test]
fn each_spawn_gets_the_next_free_name() {
    let model = model(SPAWNING);
    let trace = run(
        &model,
        r#"scenario: spawn twice
instances:
  o1: { machine: Order, fields: { orderId: 1 } }
  o2: { machine: Order, fields: { orderId: 2 } }
steps:
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
  - { source: PaymentGateway, fire: Order.capture_ok, target: o2 }
"#,
    );
    let spawns: Vec<String> = lines(&model, &trace).into_iter().filter(|l| l.contains("spawn")).collect();
    assert_eq!(spawns, strs(&["Fulfillment spawn shipment1", "Fulfillment spawn shipment2"]));
    // Each spawned instance got its own order's id, so each handoff found it.
    assert_eq!(finals(&model, &trace), ["o1: paid", "o2: paid", "shipment1: shipped", "shipment2: shipped"]);
}
