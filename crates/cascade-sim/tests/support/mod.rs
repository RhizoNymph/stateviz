//! Helpers for the play-session tests: building actions and driving a
//! session, panicking with the error when a step that should work fails.

#![allow(dead_code)]

use std::collections::BTreeMap;

use cascade_core::Model;
use cascade_core::definition::TriggerRef;
use cascade_sim::{ActionOutcome, PlayAction, PlaySession, Scenario, SimError, Trace};

pub const RACE: &str = include_str!("../fixtures/race/cascade.yaml");
pub const RACE_SCENARIO: &str = include_str!("../fixtures/race/scenario.yaml");
pub const ORDERS: &str = include_str!("../../../../examples/order-fulfillment/cascade.yaml");
pub const HAPPY: &str = include_str!("../../../../examples/order-fulfillment/scenarios/happy-path.yaml");
pub const TIMEOUT_RACE: &str = include_str!("../../../../examples/order-fulfillment/scenarios/timeout-race.yaml");
pub const TIMEOUT_FIRST: &str = include_str!("../../../../examples/order-fulfillment/scenarios/timeout-first.yaml");
pub const SECOND_FIRST: &str = include_str!("../../../../examples/order-fulfillment/scenarios/second-order-first.yaml");

/// An unbounded ping-pong: every `go` emits `Tick`, which fires `go` again.
pub const PING: &str = r#"
machines:
  Ping:
    states: [a, b]
    transitions:
      - { from: a, to: b, on: go, emits: [Tick] }
      - { from: b, to: a, on: go, emits: [Tick] }
controllers:
  Loop:
    on:
      Tick: { fire: Ping.go }
external:
  Starter: [Ping.go]
"#;

/// Orders whose payment spawns a shipment.
pub const SPAWNING: &str = r#"
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
    states: [idle, picking]
    transitions:
      - { from: idle, to: picking, on: start }
controllers:
  Fulfillment:
    on:
      OrderPaid:
        - fire: Shipment.start
          target: new Shipment with orderId = event.orderId
external:
  PaymentGateway: [Order.capture_ok]
  Warehouse: [Shipment.start]
"#;

pub fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

pub fn add(name: &str, machine: &str, fields: &[(&str, &str)]) -> PlayAction {
    PlayAction::AddInstance {
        name: Some(name.to_owned()),
        machine: machine.to_owned(),
        fields: map(fields),
        state: None,
    }
}

pub fn add_in(name: &str, machine: &str, state: &str) -> PlayAction {
    PlayAction::AddInstance {
        name: Some(name.to_owned()),
        machine: machine.to_owned(),
        fields: BTreeMap::new(),
        state: Some(state.to_owned()),
    }
}

pub fn add_auto(machine: &str) -> PlayAction {
    PlayAction::AddInstance { name: None, machine: machine.to_owned(), fields: BTreeMap::new(), state: None }
}

pub fn remove(name: &str) -> PlayAction {
    PlayAction::RemoveInstance { name: name.to_owned() }
}

pub fn trigger(text: &str) -> TriggerRef {
    match text.split_once('.') {
        Some((machine, trigger)) => TriggerRef { machine: machine.to_owned(), trigger: trigger.to_owned() },
        None => panic!("`{text}` is not Machine.trigger"),
    }
}

pub fn fire(source: &str, fired: &str, target: &str) -> PlayAction {
    fire_with(source, fired, target, &[])
}

pub fn fire_with(source: &str, fired: &str, target: &str, payload: &[(&str, &str)]) -> PlayAction {
    PlayAction::Fire {
        source: source.to_owned(),
        trigger: trigger(fired),
        target: target.to_owned(),
        payload: map(payload),
    }
}

pub fn step() -> PlayAction {
    PlayAction::Step { choice: None }
}

pub fn choose(position: u32) -> PlayAction {
    PlayAction::Step { choice: Some(position) }
}

pub fn run_all() -> PlayAction {
    PlayAction::RunUntilQuiet
}

pub fn apply(session: &mut PlaySession, model: &Model, action: PlayAction) -> ActionOutcome {
    match session.apply(model, action.clone()) {
        Ok(outcome) => outcome,
        Err(err) => panic!("expected {action:?} to apply:\n{err}"),
    }
}

pub fn apply_all(session: &mut PlaySession, model: &Model, actions: &[PlayAction]) {
    for action in actions {
        apply(session, model, action.clone());
    }
}

pub fn apply_err(session: &mut PlaySession, model: &Model, action: PlayAction) -> SimError {
    match session.apply(model, action.clone()) {
        Ok(outcome) => panic!("expected {action:?} to fail, got {outcome:?}"),
        Err(err) => err,
    }
}

/// A session with `actions` applied.
pub fn session(model: &Model, actions: &[PlayAction]) -> PlaySession {
    let mut session = PlaySession::new(model);
    apply_all(&mut session, model, actions);
    session
}

pub fn from_scenario(model: &Model, scenario: &Scenario) -> PlaySession {
    match PlaySession::from_scenario(model, scenario) {
        Ok(session) => session,
        Err(err) => panic!("expected the scenario to start a session:\n{err}"),
    }
}

/// The trace without its display name, for comparing a session with a batch
/// run of a differently named scenario.
pub fn unnamed(trace: &Trace) -> Trace {
    Trace { scenario: String::new(), ordering: None, ..trace.clone() }
}
