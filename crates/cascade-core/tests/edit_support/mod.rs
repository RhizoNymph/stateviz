//! Shared fixtures and assertions for the edit tests.

#![allow(dead_code)]

use cascade_core::definition::{
    ControllerDef, Definition, EventDef, ExternalDef, HandlerDef, MachineDef, RuleDef, StateDef, StateKindDef,
    TargetMode, TargetSpec, TransitionDef, TriggerRef,
};
use cascade_core::edit::{Applied, EditError, EditOp, apply, without_spans};
use cascade_core::{Spanned, parse_definition, resolve};

pub const ORDER_FULFILLMENT: &str = include_str!("../../../../examples/order-fulfillment/cascade.yaml");
pub const SHOP: &str = include_str!("../../../../examples/shop/cascade.yaml");

/// Nested states with local names that invite collisions: bare references
/// (`fetching`, `computing`, `hist`, `waiting`), dotted paths, a multi-source
/// `from`, duplicate transitions (ordinals) and compound initials.
pub const NESTED: &str = r#"
system: Jobs
machines:
  Job:
    initial: queued
    fields: [jobId]
    states:
      - queued
      - running:
          initial: fetching
          states:
            - fetching
            - computing
            - hist: { kind: history }
      - paused:
          states: [waiting, resuming]
      - failed
      - done: { kind: final }
    transitions:
      - { from: queued, to: running, on: start }
      - { from: fetching, to: computing, on: fetched }
      - { from: running, to: failed, on: crash, emits: [JobFailed] }
      - { from: [running.computing, waiting], to: done, on: finish, emits: [JobDone] }
      - { from: failed, to: hist, on: resume }
      - { from: fetching, to: fetching, on: retry, guard: "attempts < 3" }
      - { from: fetching, to: fetching, on: retry, guard: "attempts >= 3" }
      - { from: running, to: paused, on: pause }
      - { from: resuming, to: running.hist, on: resume }
      - { from: waiting, to: resuming, on: wake }
  Monitor:
    initial: idle
    fields: [jobId]
    states: [idle, alerting]
    transitions:
      - { from: idle, to: alerting, on: alert, emits: [Alerted] }
      - { from: alerting, to: idle, on: ack }
controllers:
  Watch:
    on:
      JobFailed:
        - fire: Monitor.alert
          target: Monitor where jobId == event.jobId
      JobDone:
        - fire: Monitor.ack
external:
  Operator: [Job.start, Job.pause, Job.wake]
  Pager: [Monitor.ack]
"#;

/// Strict events with exactly two declarations, both referenced.
pub const TWO_EVENTS: &str = r#"
events:
  Ping: { payload: [id] }
  Pong: {}
machines:
  A:
    states: [x, y]
    transitions:
      - { from: x, to: y, on: go, emits: [Ping] }
      - { from: y, to: x, on: back, emits: [Pong] }
"#;

/// Strict events with a single declaration.
pub const ONE_EVENT: &str = r#"
events:
  Ping: { payload: [id] }
machines:
  A:
    states: [x, y]
    transitions:
      - { from: x, to: y, on: go, emits: [Ping] }
controllers:
  C:
    on:
      Ping:
        - fire: A.go
"#;

pub fn fixtures() -> Vec<(&'static str, Definition)> {
    vec![
        ("order-fulfillment", parse(ORDER_FULFILLMENT)),
        ("shop", parse(SHOP)),
        ("nested", parse(NESTED)),
        ("two-events", parse(TWO_EVENTS)),
        ("one-event", parse(ONE_EVENT)),
    ]
}

pub fn parse(text: &str) -> Definition {
    let def = match parse_definition(text) {
        Ok(def) => def,
        Err(err) => panic!("fixture should parse:\n{err}"),
    };
    if let Err(err) = resolve(def.clone()) {
        panic!("fixture should resolve:\n{err}");
    }
    def
}

/// Apply `op`, which must succeed, and check the inverse law and that the
/// result resolves.
pub fn ok(def: &Definition, op: EditOp) -> Applied {
    let applied = match apply(def, &op) {
        Ok(applied) => applied,
        Err(err) => panic!("expected {op:?} to apply, got: {err}"),
    };
    assert_resolves(&applied.definition);
    assert_inverse(def, &applied);
    applied
}

/// Apply `op`, which must be rejected.
pub fn rejected(def: &Definition, op: EditOp) -> EditError {
    match apply(def, &op) {
        Ok(applied) => panic!("expected {op:?} to be rejected, got {:#?}", applied.definition),
        Err(err) => err,
    }
}

pub fn assert_resolves(def: &Definition) {
    if let Err(err) = resolve(def.clone()) {
        panic!("edited definition should resolve:\n{err}");
    }
}

/// `apply(apply(d, op).definition, inverse).definition == d`, ignoring spans.
pub fn assert_inverse(original: &Definition, applied: &Applied) {
    let undone = match apply(&applied.definition, &applied.inverse) {
        Ok(undone) => undone,
        Err(err) => panic!("inverse {:?} should apply, got: {err}", applied.inverse),
    };
    assert_same(&undone.definition, original, "undo should restore the original");
    // Redo after undo reproduces the edit.
    let redone = match apply(&undone.definition, &undone.inverse) {
        Ok(redone) => redone,
        Err(err) => panic!("redo {:?} should apply, got: {err}", undone.inverse),
    };
    assert_same(&redone.definition, &applied.definition, "redo should reproduce the edit");
}

pub fn assert_same(actual: &Definition, expected: &Definition, context: &str) {
    let (actual, expected) = (without_spans(actual), without_spans(expected));
    if actual != expected {
        panic!("{context}\n--- actual ---\n{actual:#?}\n--- expected ---\n{expected:#?}");
    }
}

// --- Lookups -------------------------------------------------------------

pub fn machine<'a>(def: &'a Definition, name: &str) -> &'a MachineDef {
    match def.machines.iter().find(|m| m.name.value == name) {
        Some(m) => m,
        None => panic!("no machine {name}"),
    }
}

pub fn has_machine(def: &Definition, name: &str) -> bool {
    def.machines.iter().any(|m| m.name.value == name)
}

pub fn state<'a>(def: &'a Definition, machine_name: &str, path: &str) -> Option<&'a StateDef> {
    let mut states = &machine(def, machine_name).states;
    let mut found = None;
    for segment in path.split('.') {
        let s = states.iter().find(|s| s.name.value == segment)?;
        states = &s.states;
        found = Some(s);
    }
    found
}

/// `from -> to @ on` for every entry of a machine, with `from` lists joined by `|`.
pub fn transitions(def: &Definition, machine_name: &str) -> Vec<String> {
    machine(def, machine_name)
        .transitions
        .iter()
        .map(|t| {
            let from: Vec<&str> = t.from.iter().map(|f| f.value.as_str()).collect();
            format!("{} -> {} @ {}", from.join("|"), t.to.value, t.on.value)
        })
        .collect()
}

pub fn initial(def: &Definition, machine_name: &str) -> Option<String> {
    machine(def, machine_name).initial.as_ref().map(|i| i.value.clone())
}

pub fn controller<'a>(def: &'a Definition, name: &str) -> &'a ControllerDef {
    match def.controllers.iter().find(|c| c.name.value == name) {
        Some(c) => c,
        None => panic!("no controller {name}"),
    }
}

pub fn handler<'a>(def: &'a Definition, controller_name: &str, event: &str) -> &'a HandlerDef {
    match controller(def, controller_name).on.iter().find(|h| h.event.value == event) {
        Some(h) => h,
        None => panic!("no handler {controller_name}/{event}"),
    }
}

pub fn external<'a>(def: &'a Definition, name: &str) -> &'a ExternalDef {
    match def.external.iter().find(|e| e.name.value == name) {
        Some(e) => e,
        None => panic!("no external {name}"),
    }
}

pub fn external_triggers(def: &Definition, name: &str) -> Vec<String> {
    external(def, name).triggers.iter().map(|t| t.value.to_string()).collect()
}

pub fn event_names(def: &Definition) -> Vec<String> {
    def.events.iter().map(|e| e.name.value.clone()).collect()
}

// --- Builders --------------------------------------------------------------

pub fn s(value: &str) -> Spanned<String> {
    Spanned::synthetic(value.to_owned())
}

pub fn leaf(name: &str) -> StateDef {
    state_def(name, StateKindDef::Normal, &[])
}

pub fn state_def(name: &str, kind: StateKindDef, children: &[StateDef]) -> StateDef {
    StateDef {
        name: s(name),
        kind: Spanned::synthetic(kind),
        initial: None,
        states: children.to_vec(),
        span: cascade_core::SourceSpan::unknown(),
    }
}

pub fn transition(from: &[&str], to: &str, on: &str) -> TransitionDef {
    TransitionDef {
        from: from.iter().map(|f| s(f)).collect(),
        to: s(to),
        on: s(on),
        guard: None,
        emits: Vec::new(),
        bounded: false,
        span: cascade_core::SourceSpan::unknown(),
    }
}

pub fn emitting(mut t: TransitionDef, events: &[&str]) -> TransitionDef {
    t.emits = events.iter().map(|e| s(e)).collect();
    t
}

pub fn machine_def(name: &str, states: &[&str], transitions: Vec<TransitionDef>) -> MachineDef {
    MachineDef {
        name: s(name),
        color: None,
        domain: None,
        initial: None,
        fields: Vec::new(),
        states: states.iter().map(|n| leaf(n)).collect(),
        transitions,
        span: cascade_core::SourceSpan::unknown(),
    }
}

pub fn trigger_ref(machine_name: &str, trigger: &str) -> TriggerRef {
    TriggerRef { machine: machine_name.to_owned(), trigger: trigger.to_owned() }
}

pub fn rule(machine_name: &str, trigger: &str) -> RuleDef {
    RuleDef {
        fire: Spanned::synthetic(trigger_ref(machine_name, trigger)),
        target: None,
        when: None,
        bounded: false,
        span: cascade_core::SourceSpan::unknown(),
    }
}

pub fn targeted(mut r: RuleDef, mode: TargetMode) -> RuleDef {
    let machine_name = r.fire.value.machine.clone();
    r.target = Some(Spanned::synthetic(TargetSpec { mode, machine: machine_name, clauses: Vec::new() }));
    r
}

pub fn handler_def(event: &str, rules: Vec<RuleDef>) -> HandlerDef {
    HandlerDef { event: s(event), rules, span: cascade_core::SourceSpan::unknown() }
}

pub fn controller_def(name: &str, handlers: Vec<HandlerDef>) -> ControllerDef {
    ControllerDef { name: s(name), on: handlers, span: cascade_core::SourceSpan::unknown() }
}

pub fn external_def(name: &str, triggers: &[(&str, &str)]) -> ExternalDef {
    ExternalDef {
        name: s(name),
        triggers: triggers.iter().map(|(m, t)| Spanned::synthetic(trigger_ref(m, t))).collect(),
        span: cascade_core::SourceSpan::unknown(),
    }
}

pub fn event_def(name: &str, payload: &[&str]) -> EventDef {
    EventDef {
        name: s(name),
        payload: payload.iter().map(|p| s(p)).collect(),
        span: cascade_core::SourceSpan::unknown(),
    }
}
