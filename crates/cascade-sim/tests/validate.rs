//! Checking a scenario against a model.

mod common;

use cascade_core::Model;
use cascade_core::span::Spanned;
use cascade_sim::{InstanceDecl, ScenarioError, ScenarioErrorKind, SimError, ValueMap, simulate, validate};
use common::{finals, has, model, scenario};

const DEFINITION: &str = r#"
machines:
  Order:
    initial: draft
    fields: [orderId]
    states:
      - draft
      - pending:
          states:
            - waiting
            - authorizing
            - back: { kind: history }
      - review:
          states: [waiting, done]
      - paid
    transitions:
      - { from: draft, to: pending, on: submit }
      - { from: pending, to: paid, on: capture_ok, emits: [OrderPaid] }
      - { from: review, to: pending.back, on: resume }
  Shipment:
    initial: idle
    states: [idle, picking]
    transitions:
      - { from: idle, to: picking, on: start }
  Log:
    states: [ready]
    transitions:
      - { from: ready, to: ready, on: tick }
controllers:
  Fulfillment:
    on:
      OrderPaid:
        - fire: Shipment.start
          target: new Shipment with orderId = event.orderId
external:
  Customer: [Order.submit]
  PaymentGateway: [Order.capture_ok]
  Warehouse: [Shipment.start]
  Clock: [Log.tick, Order.resume]
"#;

fn m() -> Model {
    model(DEFINITION)
}

fn invalid(text: &str) -> ScenarioError {
    match validate(&m(), &scenario(text)) {
        Ok(_) => panic!("expected validation errors for:\n{text}"),
        Err(err) => err,
    }
}

#[test]
fn a_valid_scenario_resolves() {
    let resolved = validate(
        &m(),
        &scenario(
            r#"scenario: ok
instances:
  o1: { machine: Order, fields: { orderId: 1 } }
  log: Log
steps:
  - { source: Customer, fire: Order.submit, target: o1 }
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
  - { source: Clock, fire: Log.tick }
"#,
        ),
    );
    let resolved = match resolved {
        Ok(r) => r,
        Err(err) => panic!("{err}"),
    };
    assert_eq!(resolved.name(), "ok");
    assert_eq!(resolved.instance_count(), 2);
    assert_eq!(resolved.step_count(), 3);
}

#[test]
fn starting_states_are_entered_by_default_entry() {
    let model = m();
    let trace = common::run(
        &model,
        r#"scenario: starts
instances:
  a: Order
  b: { machine: Order, state: pending }
  c: { machine: Order, state: authorizing }
  d: { machine: Order, state: review.waiting }
steps: []
"#,
    );
    assert_eq!(
        finals(&model, &trace),
        ["a: draft", "b: pending.waiting", "c: pending.authorizing", "d: review.waiting"]
    );
}

#[test]
fn unknown_machine_is_reported_once() {
    let err = invalid(
        "scenario: x\ninstances:\n  o1: { machine: Ordr }\nsteps:\n  - { source: Customer, fire: Order.submit, target: o1 }\n",
    );
    assert_eq!(err.diagnostics.len(), 1, "{err}");
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownMachine { name } if name == "Ordr")));
    assert_eq!(err.diagnostics[0].span.start.line, 3);
}

#[test]
fn undeclared_fields() {
    let err = invalid("scenario: x\ninstances:\n  o1: { machine: Order, fields: { orderID: 1 } }\nsteps: []\n");
    assert!(has(&err, |k| matches!(
        k,
        ScenarioErrorKind::UnknownField { machine, field, declared }
            if machine == "Order" && field == "orderID" && declared == &["orderId".to_owned()]
    )));
    // Machines that declare no fields accept any.
    assert!(
        validate(
            &m(),
            &scenario("scenario: x\ninstances:\n  s1: { machine: Shipment, fields: { any: 1 } }\nsteps: []\n")
        )
        .is_ok()
    );
}

#[test]
fn starting_state_problems() {
    let err = invalid("scenario: x\ninstances:\n  o1: { machine: Order, state: shipped }\nsteps: []\n");
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownState { name, .. } if name == "shipped")));

    let err = invalid("scenario: x\ninstances:\n  o1: { machine: Order, state: waiting }\nsteps: []\n");
    assert!(has(&err, |k| matches!(
        k,
        ScenarioErrorKind::AmbiguousState { candidates, .. }
            if candidates == &["pending.waiting".to_owned(), "review.waiting".to_owned()]
    )));

    let err = invalid("scenario: x\ninstances:\n  o1: { machine: Order, state: pending.back }\nsteps: []\n");
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::HistoryStart { state, .. } if state == "pending.back")));
    assert_eq!(err.diagnostics[0].span.start.line, 3);
}

#[test]
fn source_and_trigger_problems() {
    let base = "scenario: x\ninstances:\n  o1: Order\nsteps:\n";

    let err = invalid(&format!("{base}  - {{ source: Courier, fire: Order.submit, target: o1 }}\n"));
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownSource { name } if name == "Courier")));

    let err = invalid(&format!("{base}  - {{ source: Customer, fire: Ordr.submit, target: o1 }}\n"));
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownMachine { name } if name == "Ordr")));

    let err = invalid(&format!("{base}  - {{ source: Customer, fire: Order.cancel, target: o1 }}\n"));
    assert!(has(&err, |k| matches!(
        k,
        ScenarioErrorKind::UnknownTrigger { machine, trigger } if machine == "Order" && trigger == "cancel"
    )));

    let err = invalid(&format!("{base}  - {{ source: Customer, fire: Order.capture_ok, target: o1 }}\n"));
    assert!(has(&err, |k| matches!(
        k,
        ScenarioErrorKind::SourceCannotFire { source_name, trigger }
            if source_name == "Customer" && trigger == "Order.capture_ok"
    )));
    assert_eq!(err.diagnostics[0].span.start.line, 5);
}

#[test]
fn target_problems() {
    let base = "scenario: x\ninstances:\n  o1: Order\n  o2: Order\nsteps:\n";

    let err = invalid(&format!("{base}  - {{ source: Customer, fire: Order.submit, target: o9 }}\n"));
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownInstance { name } if name == "o9")));

    let err = invalid(&format!("{base}  - {{ source: Warehouse, fire: Shipment.start, target: o1 }}\n"));
    assert!(has(&err, |k| matches!(
        k,
        ScenarioErrorKind::TargetMachineMismatch { instance, machine, fire }
            if instance == "o1" && machine == "Order" && fire == "Shipment.start"
    )));

    let err = invalid(&format!("{base}  - {{ source: Customer, fire: Order.submit }}\n"));
    assert!(has(&err, |k| matches!(
        k,
        ScenarioErrorKind::AmbiguousInstance { machine, candidates }
            if machine == "Order" && candidates == &["o1".to_owned(), "o2".to_owned()]
    )));

    let err = invalid("scenario: x\nsteps:\n  - { source: Customer, fire: Order.submit }\n");
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::NoInstance { machine } if machine == "Order")));
}

#[test]
fn names_a_spawn_rule_could_create_are_accepted() {
    // Fulfillment spawns Shipments, so `shipment1` may exist by the time the
    // step runs.
    let ok = "scenario: x\nsteps:\n  - { source: Warehouse, fire: Shipment.start, target: shipment1 }\n";
    assert!(validate(&m(), &scenario(ok)).is_ok());
    // So may "the one Shipment".
    let ok = "scenario: x\nsteps:\n  - { source: Warehouse, fire: Shipment.start }\n";
    assert!(validate(&m(), &scenario(ok)).is_ok());

    for bad in ["shipment", "shipmentX", "Shipment1", "order1"] {
        let text = format!("scenario: x\nsteps:\n  - {{ source: Warehouse, fire: Shipment.start, target: {bad} }}\n");
        let err = invalid(&text);
        assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownInstance { .. })), "{bad}");
    }
    // Orders are never spawned.
    let err = invalid("scenario: x\nsteps:\n  - { source: Customer, fire: Order.submit, target: order1 }\n");
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownInstance { .. })));
}

#[test]
fn a_spawnable_target_that_was_never_spawned_fails_when_its_step_runs() {
    let text = "scenario: x\nsteps:\n  - { source: Warehouse, fire: Shipment.start, target: shipment1 }\n";
    match simulate(&m(), &scenario(text)) {
        Err(SimError::Scenario(err)) => {
            assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownInstance { name } if name == "shipment1")));
            assert_eq!(err.diagnostics[0].span.start.line, 3);
        }
        other => panic!("expected a scenario error, got {other:?}"),
    }
    let text = "scenario: x\nsteps:\n  - { source: Warehouse, fire: Shipment.start }\n";
    match simulate(&m(), &scenario(text)) {
        Err(SimError::Scenario(err)) => {
            assert!(has(&err, |k| matches!(k, ScenarioErrorKind::NoInstance { machine } if machine == "Shipment")));
        }
        other => panic!("expected a scenario error, got {other:?}"),
    }
}

#[test]
fn duplicate_instances_built_in_code() {
    let decl = |name: &str| InstanceDecl {
        name: Spanned::synthetic(name.to_owned()),
        machine: Spanned::synthetic("Order".to_owned()),
        fields: ValueMap::new(),
        state: None,
        span: cascade_core::span::SourceSpan::unknown(),
    };
    let mut s = scenario("scenario: x\nsteps: []\n");
    s.instances = vec![decl("o1"), decl("o1")];
    let err = match validate(&m(), &s) {
        Ok(_) => panic!("expected a duplicate"),
        Err(err) => err,
    };
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::DuplicateInstance { name } if name == "o1")));
}

#[test]
fn every_problem_is_reported_in_source_order() {
    let err = invalid(
        r#"scenario: many
instances:
  o1: { machine: Order, fields: { nope: 1 } }
  s1: { machine: Shipment, state: gone }
steps:
  - { source: Courier, fire: Order.submit, target: o1 }
  - { source: Customer, fire: Order.capture_ok, target: o1 }
  - { source: Customer, fire: Order.submit, target: zz }
"#,
    );
    let lines: Vec<u32> = err.diagnostics.iter().map(|d| d.span.start.line).collect();
    assert_eq!(lines, [3, 4, 6, 7, 8], "{err}");
}

#[test]
fn a_spawn_can_make_a_targetless_step_ambiguous_when_it_runs() {
    let text = r#"scenario: x
instances:
  o1: { machine: Order, state: pending, fields: { orderId: 1 } }
  s: Shipment
steps:
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
  - { source: Warehouse, fire: Shipment.start }
"#;
    // One Shipment is declared, so the step is fine on paper...
    assert!(validate(&m(), &scenario(text)).is_ok());
    // ...but by the time it runs, Fulfillment has spawned a second one.
    match simulate(&m(), &scenario(text)) {
        Err(SimError::Scenario(err)) => {
            assert!(has(&err, |k| matches!(
                k,
                ScenarioErrorKind::AmbiguousInstance { machine, candidates }
                    if machine == "Shipment" && candidates == &["s".to_owned(), "shipment1".to_owned()]
            )));
            assert_eq!(err.diagnostics[0].span.start.line, 7);
        }
        other => panic!("expected a scenario error, got {other:?}"),
    }
}
