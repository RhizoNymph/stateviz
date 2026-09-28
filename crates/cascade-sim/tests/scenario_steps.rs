//! The scenario steps that go beyond external fires: delivering a chosen
//! queue item (`- step`, `- { step: n }`), running until quiet (`- run`),
//! creating and removing instances mid-run, and ending without draining
//! (`end: pause`).

mod common;
mod support;

use cascade_sim::{
    Directive, ScenarioEnd, ScenarioEntry, ScenarioErrorKind, SimError, StepTiming, simulate, simulate_run, validate,
};
use common::{finals, has, lifelines, lines, model, scenario, scenario_err, strs};
use support::{RACE, SPAWNING};

/// One word per entry, in file order.
fn entry_names(s: &cascade_sim::Scenario) -> Vec<String> {
    s.entries()
        .map(|entry| match entry {
            ScenarioEntry::Fire(step) => format!("fire {}", step.fire.value),
            ScenarioEntry::Directive(Directive::Deliver { choice: None, .. }) => "step".to_owned(),
            ScenarioEntry::Directive(Directive::Deliver { choice: Some(n), .. }) => format!("step {n}"),
            ScenarioEntry::Directive(Directive::Run { .. }) => "run".to_owned(),
            ScenarioEntry::Directive(Directive::Create(decl)) => format!("create {}", decl.name.value),
            ScenarioEntry::Directive(Directive::Remove { name, .. }) => format!("remove {}", name.value),
        })
        .collect()
}

const MANUAL: &str = r#"scenario: manual
instances:
  o1: { machine: Order, fields: { orderId: 1 } }
steps:
  - step
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
  - { step: 1 }
  - run
  - { create: s2, machine: Shipment, fields: { orderId: 2 }, state: idle }
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1, timing: immediate }
  - { remove: s2 }
end: pause
"#;

#[test]
fn every_step_kind_parses_in_file_order() {
    let s = scenario(MANUAL);
    assert_eq!(
        entry_names(&s),
        strs(&["step", "fire Order.capture_ok", "step 1", "run", "create s2", "fire Order.capture_ok", "remove s2",])
    );
    // External fires are still `steps`, each carrying the steps written
    // before it; what follows the last fire is `trailing`.
    assert_eq!(s.steps.len(), 2);
    assert_eq!(s.steps[0].before.len(), 1);
    assert_eq!(s.steps[1].before.len(), 3);
    assert_eq!(s.steps[1].timing, StepTiming::Immediate);
    assert_eq!(s.trailing.len(), 1);
    assert_eq!(s.end, ScenarioEnd::Pause);

    let Some(Directive::Create(decl)) = s.steps[1].before.get(2) else {
        panic!("expected a create, got {:?}", s.steps[1].before);
    };
    assert_eq!(decl.machine.value, "Shipment");
    assert_eq!(decl.fields.get("orderId"), Some("2"));
    assert_eq!(decl.state.as_ref().map(|s| s.value.as_str()), Some("idle"));
    assert_eq!(decl.span.start.line, 9);
}

#[test]
fn directive_spans_point_at_their_lines() {
    let s = scenario(MANUAL);
    let lines: Vec<u32> = s
        .entries()
        .map(|entry| match entry {
            ScenarioEntry::Fire(step) => step.span.start.line,
            ScenarioEntry::Directive(d) => d.span().start.line,
        })
        .collect();
    assert_eq!(lines, [5, 6, 7, 8, 9, 10, 11]);
}

#[test]
fn plain_scenarios_have_no_directives_and_drain_at_the_end() {
    let s = scenario(support::HAPPY);
    assert_eq!(s.end, ScenarioEnd::Drain);
    assert!(s.trailing.is_empty());
    assert!(s.steps.iter().all(|step| step.before.is_empty()));
    assert_eq!(s.entries().count(), s.steps.len());

    let s = scenario("scenario: x\nsteps: []\nend: drain\n");
    assert_eq!(s.end, ScenarioEnd::Drain);
}

#[test]
fn malformed_directives_are_reported() {
    for bad in ["-1", "x", "1.5", "~"] {
        let err = scenario_err(&format!("scenario: a\nsteps:\n  - {{ step: {bad} }}\n"));
        assert!(has(&err, |k| matches!(k, ScenarioErrorKind::InvalidQueuePosition { .. })), "{bad}: {err}");
        assert_eq!(err.diagnostics[0].span.start.line, 3);
    }

    let err = scenario_err("scenario: a\nsteps:\n  - { step: 1, target: o1 }\n");
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownKey { key, .. } if key == "target")), "{err}");

    let err = scenario_err("scenario: a\nsteps:\n  - { create: o2 }\n");
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::MissingKey { key, .. } if key == "machine")), "{err}");

    let err = scenario_err("scenario: a\nsteps:\n  - { create: o2, machine: Order, payload: { x: 1 } }\n");
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownKey { key, .. } if key == "payload")), "{err}");

    let err = scenario_err("scenario: a\nsteps:\n  - { remove: 'o 2' }\n");
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::InvalidName { name, .. } if name == "o 2")), "{err}");

    let err = scenario_err("scenario: a\nsteps:\n  - { remove: o2, now: yes }\n");
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownKey { key, .. } if key == "now")), "{err}");

    let err = scenario_err("scenario: a\nsteps: []\nend: later\n");
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownEnd { text } if text == "later")), "{err}");

    // A scalar that is not a step keyword is still a misplaced mapping.
    let err = scenario_err("scenario: a\nsteps:\n  - walk\n");
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::WrongType { .. })), "{err}");
}

// --- Validation ---------------------------------------------------------------

const TWO_ORDERS: &str = r#"
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
      - { from: idle, to: picking, on: start }
      - { from: idle, to: on_hold, on: hold }
controllers:
  Fulfillment:
    on:
      OrderPaid:
        - fire: Shipment.start
          target: Shipment where orderId == event.orderId
external:
  PaymentGateway: [Order.capture_ok]
  Warehouse: [Shipment.start]
"#;

fn invalid(text: &str) -> cascade_sim::ScenarioError {
    match validate(&model(TWO_ORDERS), &scenario(text)) {
        Ok(_) => panic!("expected validation errors for:\n{text}"),
        Err(err) => err,
    }
}

#[test]
fn created_instances_can_be_targeted_after_their_create() {
    let ok = r#"scenario: x
steps:
  - { create: o1, machine: Order, fields: { orderId: 1 } }
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
  - { source: PaymentGateway, fire: Order.capture_ok }
"#;
    assert!(validate(&model(TWO_ORDERS), &scenario(ok)).is_ok());

    let err = invalid(
        "scenario: x\nsteps:\n  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }\n  - { create: o1, machine: Order }\n",
    );
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownInstance { name } if name == "o1")), "{err}");
    assert_eq!(err.diagnostics[0].span.start.line, 3);

    let err = invalid(
        "scenario: x\nsteps:\n  - { create: s1, machine: Shipment }\n  - { source: PaymentGateway, fire: Order.capture_ok, target: s1 }\n",
    );
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::TargetMachineMismatch { .. })), "{err}");
}

#[test]
fn created_instances_are_checked_like_declared_ones() {
    let err = invalid("scenario: x\nsteps:\n  - { create: z1, machine: Zeppelin }\n");
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownMachine { name } if name == "Zeppelin")));

    let err = invalid("scenario: x\nsteps:\n  - { create: o1, machine: Order, fields: { colour: red } }\n");
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownField { field, .. } if field == "colour")));

    let err = invalid("scenario: x\nsteps:\n  - { create: o1, machine: Order, state: gone }\n");
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownState { name, .. } if name == "gone")));
}

#[test]
fn instance_names_are_never_reused() {
    // Not even after the first holder was removed.
    let err = invalid(
        "scenario: x\ninstances:\n  o1: Order\nsteps:\n  - { remove: o1 }\n  - { create: o1, machine: Order }\n",
    );
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::DuplicateInstance { name } if name == "o1")), "{err}");
    assert_eq!(err.diagnostics[0].span.start.line, 6);
}

#[test]
fn removed_instances_cannot_be_targeted_or_removed_again() {
    let err = invalid(
        "scenario: x\ninstances:\n  o1: Order\nsteps:\n  - { remove: o1 }\n  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }\n",
    );
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownInstance { name } if name == "o1")), "{err}");

    let err = invalid("scenario: x\ninstances:\n  o1: Order\nsteps:\n  - { remove: o1 }\n  - { remove: o1 }\n");
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownInstance { name } if name == "o1")), "{err}");
    assert_eq!(err.diagnostics[0].span.start.line, 6);

    let err = invalid("scenario: x\nsteps:\n  - { remove: nobody }\n");
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownInstance { name } if name == "nobody")), "{err}");
}

#[test]
fn a_targetless_step_counts_the_instances_alive_at_that_point() {
    // Two orders, but one is gone by the time the step runs.
    let ok = r#"scenario: x
instances:
  o1: Order
  o2: Order
steps:
  - { remove: o2 }
  - { source: PaymentGateway, fire: Order.capture_ok }
"#;
    assert!(validate(&model(TWO_ORDERS), &scenario(ok)).is_ok());

    // One order, but a second one is created before the step.
    let err = invalid(
        "scenario: x\ninstances:\n  o1: Order\nsteps:\n  - { create: o2, machine: Order }\n  - { source: PaymentGateway, fire: Order.capture_ok }\n",
    );
    assert!(has(&err, |k| matches!(
        k,
        ScenarioErrorKind::AmbiguousInstance { candidates, .. } if candidates == &["o1".to_owned(), "o2".to_owned()]
    )));
}

// --- Running ------------------------------------------------------------------

const RACE_INSTANCES: &str = "instances:\n  o1: { machine: Order, fields: { orderId: \"1\" } }\n  s1: { machine: Shipment, fields: { orderId: \"1\" } }\n";

fn race_scenario(steps: &str) -> String {
    format!(
        "scenario: manual race\n{RACE_INSTANCES}steps:\n  - {{ source: PaymentGateway, fire: Order.capture_ok, target: o1 }}\n{steps}"
    )
}

#[test]
fn choosing_a_queue_item_swaps_the_race() {
    let model = model(RACE);
    let trace = match simulate(&model, &scenario(&race_scenario("  - step\n  - { step: 1 }\n"))) {
        Ok(trace) => trace,
        Err(err) => panic!("{err}"),
    };
    assert_eq!(
        lines(&model, &trace),
        strs(&[
            "ext PaymentGateway -> o1 capture_ok",
            "o1 pending -> paid",
            "o1 emit OrderPaid",
            "Fulfillment <- OrderPaid",
            "Fulfillment fire s1 start",
            "Billing <- OrderPaid",
            "Billing fire s1 hold",
            "s1 idle -> on_hold",
            // The final drain delivers what is left.
            "s1 drop start @ on_hold",
        ])
    );
    assert_eq!(finals(&model, &trace), ["o1: paid", "s1: on_hold"]);
}

#[test]
fn step_zero_and_step_are_the_head() {
    let model = model(RACE);
    let plain = simulate(&model, &scenario(&race_scenario(""))).map(|t| lines(&model, &t));
    let head = simulate(&model, &scenario(&race_scenario("  - step\n  - { step: 0 }\n"))).map(|t| lines(&model, &t));
    assert_eq!(plain, head);
}

#[test]
fn end_pause_leaves_the_queue_undelivered() {
    let model = model(RACE);
    let trace = match simulate(&model, &scenario(&race_scenario("  - step\nend: pause\n"))) {
        Ok(trace) => trace,
        Err(err) => panic!("{err}"),
    };
    assert_eq!(lines(&model, &trace).len(), 7);
    assert_eq!(lines(&model, &trace).last().map(String::as_str), Some("Billing fire s1 hold"));
    assert_eq!(finals(&model, &trace), ["o1: paid", "s1: idle"]);
}

#[test]
fn run_drains_before_later_immediate_steps() {
    let model = model(RACE);
    let with_run = race_scenario(
        "  - run\n  - { source: PaymentGateway, fire: Order.capture_ok, target: o1, timing: immediate }\n",
    );
    let trace = match simulate(&model, &scenario(&with_run)) {
        Ok(trace) => trace,
        Err(err) => panic!("{err}"),
    };
    let got = lines(&model, &trace);
    assert_eq!(
        got[7..],
        strs(&[
            "s1 idle -> picking",
            "s1 drop hold @ picking",
            "ext PaymentGateway -> o1 capture_ok",
            "o1 drop capture_ok @ paid",
        ])
    );
}

#[test]
fn a_created_instance_takes_part_from_its_create_on() {
    let model = model(RACE);
    let text = r#"scenario: late shipment
instances:
  o1: { machine: Order, fields: { orderId: 1 } }
steps:
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
  - { create: s1, machine: Shipment, fields: { orderId: 1 } }
"#;
    let trace = match simulate(&model, &scenario(text)) {
        Ok(trace) => trace,
        Err(err) => panic!("{err}"),
    };
    // OrderPaid was queued before s1 existed but delivered after, so both
    // controllers find it.
    assert_eq!(
        lines(&model, &trace)[3..],
        strs(&[
            "Fulfillment <- OrderPaid",
            "Fulfillment fire s1 start",
            "Billing <- OrderPaid",
            "Billing fire s1 hold",
            "s1 idle -> picking",
            "s1 drop hold @ picking",
        ])
    );
    assert_eq!(
        lifelines(&model, &trace),
        strs(&["ext:PaymentGateway", "Order:o1", "Shipment:s1", "ctl:Fulfillment", "ctl:Billing"])
    );
}

#[test]
fn removing_an_instance_discards_the_fires_queued_for_it() {
    let model = model(RACE);
    let trace = match simulate(&model, &scenario(&race_scenario("  - step\n  - { remove: s1 }\n"))) {
        Ok(trace) => trace,
        Err(err) => panic!("{err}"),
    };
    // The two fires at s1 were queued, then dropped from the queue with it:
    // nothing delivers them.
    assert_eq!(lines(&model, &trace).last().map(String::as_str), Some("Billing fire s1 hold"));
    // Its lifeline and last state stay in the trace.
    assert_eq!(finals(&model, &trace), ["o1: paid", "s1: idle"]);
}

#[test]
fn queue_problems_are_reported_where_the_step_is() {
    let model = model(RACE);
    match simulate(&model, &scenario(&race_scenario("  - step\n  - { step: 2 }\n"))) {
        Err(SimError::Scenario(err)) => {
            assert!(has(&err, |k| matches!(k, ScenarioErrorKind::NoPendingItem { position: 2, pending: 2 })), "{err}");
            assert_eq!(err.diagnostics[0].span.start.line, 8);
        }
        other => panic!("expected a scenario error, got {other:?}"),
    }
    match simulate(&model, &scenario(&race_scenario("  - run\n  - step\n"))) {
        Err(SimError::Scenario(err)) => {
            assert!(has(&err, |k| matches!(k, ScenarioErrorKind::QueueEmpty)), "{err}");
            assert_eq!(err.diagnostics[0].span.start.line, 8);
        }
        other => panic!("expected a scenario error, got {other:?}"),
    }
}

#[test]
fn creating_a_name_a_spawn_took_fails_when_it_runs() {
    let model = model(SPAWNING);
    let text = r#"scenario: clash
instances:
  o1: { machine: Order, fields: { orderId: 1 } }
steps:
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
  - run
  - { create: shipment1, machine: Shipment }
"#;
    match simulate(&model, &scenario(text)) {
        Err(SimError::Scenario(err)) => {
            assert!(has(&err, |k| matches!(k, ScenarioErrorKind::NameTaken { name } if name == "shipment1")), "{err}");
            assert_eq!(err.diagnostics[0].span.start.line, 7);
        }
        other => panic!("expected a scenario error, got {other:?}"),
    }
}

#[test]
fn spawned_instances_can_be_removed_by_name() {
    let model = model(SPAWNING);
    let text = r#"scenario: spawn then remove
instances:
  o1: { machine: Order, fields: { orderId: 1 } }
steps:
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
  - run
  - { remove: shipment1 }
  - { create: shipment2, machine: Shipment }
"#;
    let run = match simulate_run(&model, &scenario(text)) {
        Ok(run) => run,
        Err(err) => panic!("{err}"),
    };
    assert_eq!(finals(&model, &run.trace), ["o1: paid", "shipment1: picking", "shipment2: idle"]);

    // Removing a spawn name that was never spawned fails when it runs.
    let text = "scenario: x\nsteps:\n  - { remove: shipment1 }\n";
    match simulate(&model, &scenario(text)) {
        Err(SimError::Scenario(err)) => {
            assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownInstance { name } if name == "shipment1")));
            assert_eq!(err.diagnostics[0].span.start.line, 3);
        }
        other => panic!("expected a scenario error, got {other:?}"),
    }
}

#[test]
fn the_example_delivers_the_second_payment_first() {
    let model = model(support::ORDERS);
    let trace = match simulate(&model, &scenario(support::SECOND_FIRST)) {
        Ok(trace) => trace,
        Err(err) => panic!("{err}"),
    };
    let starts: Vec<String> = lines(&model, &trace).into_iter().filter(|l| l.contains("fire s")).collect();
    assert_eq!(starts, strs(&["Fulfillment fire s2 start", "Fulfillment fire s1 start"]));
    assert_eq!(finals(&model, &trace), ["o1: paid", "o2: paid", "s1: picking", "s2: picking"]);
}
