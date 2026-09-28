//! Definition → Model resolution: structure, reverse indexes, statechart
//! semantics, stable keys and every reference diagnostic.

use cascade_core::definition::FieldClause;
use cascade_core::definition::ValueExpr;
use cascade_core::model::{StateKind, Target};
use cascade_core::{DiagnosticKind, ElementKey, ElementRef, LoadError, Model, PaletteColor, load_str};

const SPEC_EXAMPLE: &str = include_str!("../../../examples/order-fulfillment/cascade.yaml");

fn load(text: &str) -> Model {
    match load_str(text) {
        Ok(model) => model,
        Err(err) => panic!("expected the definition to load:\n{err}"),
    }
}

fn load_err(text: &str) -> LoadError {
    match load_str(text) {
        Ok(_) => panic!("expected load diagnostics"),
        Err(err) => err,
    }
}

fn has(err: &LoadError, pred: impl Fn(&DiagnosticKind) -> bool) -> bool {
    err.diagnostics.iter().any(|d| pred(&d.kind))
}

#[test]
fn spec_example_resolves() {
    let model = load(SPEC_EXAMPLE);
    assert_eq!(model.machine_count(), 2);
    assert_eq!(model.transition_count(), 5);
    assert_eq!(model.controller_count(), 1);
    assert_eq!(model.external_count(), 3);

    let order = model.machine_by_name("Order").expect("Order");
    let order_m = model.machine(order);
    assert_eq!(order_m.color, Some(PaletteColor::Blue));
    assert_eq!(model.state(order_m.initial).path, "draft");
    assert_eq!(order_m.states.len(), 4);

    let paid = model.event_by_name("OrderPaid").expect("OrderPaid");
    assert!(!model.event(paid).declared);
    assert_eq!(model.event(paid).emitted_by.len(), 1);
    assert_eq!(model.event(paid).handlers.len(), 1);

    let shipment = model.machine_by_name("Shipment").expect("Shipment");
    let start = model.trigger_by_name(shipment, "start").expect("start trigger");
    assert_eq!(model.trigger(start).accepted_by.len(), 1);
    assert_eq!(model.trigger(start).fired_by.len(), 1);

    let rule = model.trigger(start).fired_by[0];
    assert_eq!(
        model.rule(rule).target,
        Target::One {
            predicates: vec![FieldClause { field: "orderId".into(), value: ValueExpr::EventField("orderId".into()) }]
        }
    );

    let timeout = model.trigger_by_name(order, "timeout").expect("timeout");
    let clock = model.external_by_name("Clock").expect("Clock");
    assert_eq!(model.trigger(timeout).sources, vec![clock]);
}

#[test]
fn transition_labels_and_spans() {
    let model = load(SPEC_EXAMPLE);
    let labels: Vec<String> = model.transition_ids().map(|t| model.transition_label(t)).collect();
    assert_eq!(
        labels,
        [
            "Order: draft → pending",
            "Order: pending → paid",
            "Order: pending → cancelled",
            "Shipment: idle → picking",
            "Shipment: picking → shipped",
        ]
    );
    // Click-to-source: the second Order transition is on line 10 of the file.
    let t = model.transition_ids().nth(1).expect("transition");
    assert_eq!(model.span_of(ElementRef::Transition(t)).line(), Some(10));
}

#[test]
fn every_element_key_round_trips() {
    let model = load(SPEC_EXAMPLE);
    for element in model.all_elements() {
        let key = model.key_of(element);
        assert_eq!(model.resolve_key(&key), Some(element), "{key}");
        let reparsed: ElementKey = key.to_string().parse().expect("key parses");
        assert_eq!(reparsed, key);
    }
}

#[test]
fn keys_survive_unrelated_edits() {
    let before = load(SPEC_EXAMPLE);
    let edited = SPEC_EXAMPLE.replace(
        "      - { from: draft,   to: pending,   on: submit }\n",
        "      - { from: draft,   to: cancelled, on: abandon }\n      - { from: draft,   to: pending,   on: submit }\n",
    );
    let after = load(&edited);
    for element in before.all_elements() {
        let key = before.key_of(element);
        let found = after.resolve_key(&key).expect("still present after edit");
        assert_eq!(after.key_of(found), key);
    }
}

const NESTED: &str = r#"
machines:
  Job:
    initial: queued
    fields: [batch]
    states:
      queued: {}
      running:
        initial: fetching
        states:
          - fetching
          - computing
          - hist: { kind: history }
      failed: {}
      done: { kind: final }
    transitions:
      - { from: queued, to: running, on: start }
      - { from: fetching, to: computing, on: fetched }
      - { from: running, to: failed, on: crash }
      - { from: running.computing, to: done, on: finish }
      - { from: failed, to: hist, on: resume }
      - { from: fetching, to: fetching, on: retry, guard: "attempts < 3" }
      - { from: fetching, to: fetching, on: retry, guard: "attempts >= 3" }
"#;

#[test]
fn nested_states_have_paths_and_structure() {
    let model = load(NESTED);
    let job = model.machine_by_name("Job").expect("Job");
    let paths: Vec<&str> = model.machine(job).states.iter().map(|&s| model.state(s).path.as_str()).collect();
    assert_eq!(paths, ["queued", "running", "running.fetching", "running.computing", "running.hist", "failed", "done"]);
    let running = model.state_by_path(job, "running").expect("running");
    let fetching = model.state_by_path(job, "running.fetching").expect("fetching");
    match &model.state(running).kind {
        StateKind::Compound { children, initial } => {
            assert_eq!(children.len(), 3);
            assert_eq!(*initial, fetching);
        }
        other => panic!("running should be compound, got {other:?}"),
    }
    assert_eq!(model.state(fetching).parent, Some(running));
    assert_eq!(model.state(fetching).depth, 1);
    assert!(model.state(model.state_by_path(job, "done").expect("done")).is_final());
    assert!(model.state(model.state_by_path(job, "running.hist").expect("hist")).is_history());
    assert_eq!(model.default_entry(running), fetching);
    assert!(model.is_ancestor_or_self(running, fetching));
    assert!(!model.is_ancestor_or_self(fetching, running));
}

#[test]
fn enabled_transitions_follow_statechart_priority() {
    let model = load(NESTED);
    let job = model.machine_by_name("Job").expect("Job");
    let fetching = model.state_by_path(job, "running.fetching").expect("fetching");
    let computing = model.state_by_path(job, "running.computing").expect("computing");
    let crash = model.trigger_by_name(job, "crash").expect("crash");
    let finish = model.trigger_by_name(job, "finish").expect("finish");
    let retry = model.trigger_by_name(job, "retry").expect("retry");

    // `crash` is declared on the parent and applies to every child.
    assert_eq!(model.enabled_transitions(fetching, crash).len(), 1);
    assert_eq!(model.enabled_transitions(computing, crash).len(), 1);
    // `finish` only leaves computing.
    assert!(model.enabled_transitions(fetching, finish).is_empty());
    assert_eq!(model.enabled_transitions(computing, finish).len(), 1);
    // Two guarded candidates; the choice is left to guards.
    assert_eq!(model.enabled_transitions(fetching, retry).len(), 2);
}

#[test]
fn duplicate_transitions_get_distinct_ordinals() {
    let model = load(NESTED);
    let keys: Vec<String> = model
        .transition_ids()
        .filter(|&t| model.trigger(model.transition(t).trigger).name == "retry")
        .map(|t| model.key_of(ElementRef::Transition(t)).to_string())
        .collect();
    assert_eq!(
        keys,
        [
            "transition:Job:running.fetching->running.fetching@retry",
            "transition:Job:running.fetching->running.fetching@retry#1",
        ]
    );
}

#[test]
fn from_list_expands_to_one_transition_per_source() {
    let model =
        load("machines:\n  A:\n    states: [x, y, z]\n    transitions:\n      - { from: [x, y], to: z, on: go }\n");
    assert_eq!(model.transition_count(), 2);
    let go = model.trigger_ids().next().expect("go");
    assert_eq!(model.trigger(go).accepted_by.len(), 2);
}

#[test]
fn declared_events_are_strict() {
    let text = r#"
machines:
  A:
    states: [x, y]
    transitions:
      - { from: x, to: y, on: go, emits: [Went, Typo] }
events:
  Went: { payload: [id] }
controllers:
  C:
    on:
      Went:
        - fire: A.go
          target: A where id == event.missing
"#;
    let err = load_err(text);
    assert!(has(&err, |k| matches!(k, DiagnosticKind::UndeclaredEvent { event } if event == "Typo")));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::UnknownPayloadField { field, .. } if field == "missing")));
}

#[test]
fn declared_payloads_are_recorded() {
    let model = load(
        "machines:\n  A:\n    states: [x, y]\n    transitions:\n      - { from: x, to: y, on: go, emits: Went }\nevents:\n  Went: { payload: [id, amount] }\n",
    );
    let went = model.event_by_name("Went").expect("Went");
    assert!(model.event(went).declared);
    assert_eq!(model.event(went).payload, ["id", "amount"]);
}

#[test]
fn reference_diagnostics() {
    let text = r#"
machines:
  Order:
    initial: nowhere
    fields: [orderId]
    states:
      a: { states: [inner] }
      b: { states: [inner] }
      done: { kind: final }
      h: { kind: history, states: [x] }
    transitions:
      - { from: inner, to: a, on: go }
      - { from: missing, to: a, on: go }
      - { from: done, to: a, on: reopen }
      - { from: h, to: a, on: back }
  Empty:
    states: []
controllers:
  C:
    on:
      E:
        - fire: Ghost.go
        - fire: Order.go
          target: Shipment where orderId == event.orderId
        - fire: Order.go
          target: Order where customer == event.customer
        - fire: Empty.go
external:
  User: [Nope.go]
"#;
    let err = load_err(text);
    assert!(has(&err, |k| matches!(k, DiagnosticKind::UnknownState { name, .. } if name == "nowhere")));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::AmbiguousState { name, candidates, .. }
        if name == "inner" && candidates == &["a.inner".to_owned(), "b.inner".to_owned()])));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::UnknownState { name, .. } if name == "missing")));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::TransitionFromFinal { state } if state == "done")));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::TransitionFromHistory { state } if state == "h")));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::ChildrenNotAllowed { state, .. } if state == "h")));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::EmptyMachine { machine } if machine == "Empty")));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::UnknownMachine { name } if name == "Ghost")));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::UnknownMachine { name } if name == "Nope")));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::TargetMachineMismatch { target, .. } if target == "Shipment")));
    assert!(has(&err, |k| matches!(k, DiagnosticKind::UnknownField { field, .. } if field == "customer")));
    // A machine that failed to resolve is not reported again as unknown.
    assert!(!has(&err, |k| matches!(k, DiagnosticKind::UnknownMachine { name } if name == "Empty")));
    // Diagnostics are sorted by position.
    let spans: Vec<_> = err.diagnostics.iter().map(|d| d.span).collect();
    let mut sorted = spans.clone();
    sorted.sort();
    assert_eq!(spans, sorted);
}

#[test]
fn initial_child_must_be_a_child() {
    let err = load_err("machines:\n  A:\n    states:\n      p: { initial: q, states: [r] }\n      q: {}\n");
    assert!(has(
        &err,
        |k| matches!(k, DiagnosticKind::InitialNotChild { parent, initial } if parent == "p" && initial == "q")
    ));
}

#[test]
fn history_cannot_be_initial() {
    let err = load_err("machines:\n  A:\n    states:\n      p: { states: [{ h: { kind: history } }, r] }\n");
    assert!(has(&err, |k| matches!(k, DiagnosticKind::InitialIsHistory { .. })));
}

#[test]
fn spawn_and_fan_out_targets() {
    let text = r#"
machines:
  Job:
    states: [idle, run]
    transitions: [{ from: idle, to: run, on: go }]
controllers:
  Sched:
    on:
      Tick:
        - { fire: Job.go, target: "all Job where batch == event.batch" }
        - { fire: Job.go, target: "new Job with batch = event.batch" }
"#;
    let model = load(text);
    let targets: Vec<&Target> = model.rules().map(|(_, r)| &r.target).collect();
    assert!(matches!(targets[0], Target::All { predicates } if predicates.len() == 1));
    assert!(matches!(targets[1], Target::Spawn { assignments } if assignments.len() == 1));
}

#[test]
fn triggers_exist_even_when_no_transition_accepts_them() {
    let model = load("machines:\n  A:\n    states: [x]\ncontrollers:\n  C:\n    on:\n      E: [{ fire: A.nothing }]\n");
    let a = model.machine_by_name("A").expect("A");
    let t = model.trigger_by_name(a, "nothing").expect("trigger created from the fire");
    assert!(model.trigger(t).accepted_by.is_empty());
    assert_eq!(model.trigger(t).fired_by.len(), 1);
}
