//! Queue semantics: FIFO delivery across controllers, step timing, dropped
//! triggers, payload propagation, cause links and the step limit.

mod common;

use cascade_core::Model;
use cascade_sim::{STEP_LIMIT, SimError, StepIx, TraceStepKind, simulate, simulate_run};
use common::{causes, finals, lines, model, run, run_err, scenario, strs};

/// Two controllers react to `OrderPaid`; a third reacts to what their
/// fires set off.
const CASCADE: &str = r#"
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
      - { from: idle, to: picking, on: start, emits: [ShipmentStarted] }
  Invoice:
    initial: open
    fields: [orderId]
    states: [open, settled]
    transitions:
      - { from: open, to: settled, on: settle, emits: [InvoiceSettled] }
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
  Billing:
    on:
      OrderPaid:
        - fire: Invoice.settle
          target: Invoice where orderId == event.orderId
  Audit:
    on:
      ShipmentStarted: { fire: Log.record }
      InvoiceSettled: { fire: Log.record }
external:
  PaymentGateway: [Order.capture_ok]
"#;

const PAID: &str = r#"scenario: paid
instances:
  o1: { machine: Order, fields: { orderId: 1 } }
  s1: { machine: Shipment, fields: { orderId: 1 } }
  i1: { machine: Invoice, fields: { orderId: 1 } }
  log: Log
steps:
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
"#;

#[test]
fn fires_from_two_controllers_interleave_in_fifo_order() {
    let model = model(CASCADE);
    let trace = run(&model, PAID);
    assert_eq!(
        lines(&model, &trace),
        strs(&[
            "ext PaymentGateway -> o1 capture_ok", // 0
            "o1 pending -> paid",                  // 1
            "o1 emit OrderPaid",                   // 2: queue [OrderPaid]
            "Fulfillment <- OrderPaid",            // 3
            "Fulfillment fire s1 start",           // 4: queue [start]
            "Billing <- OrderPaid",                // 5
            "Billing fire i1 settle",              // 6: queue [start, settle]
            "s1 idle -> picking",                  // 7
            "s1 emit ShipmentStarted",             // 8: queue [settle, ShipmentStarted]
            "i1 open -> settled",                  // 9
            "i1 emit InvoiceSettled",              // 10: queue [ShipmentStarted, InvoiceSettled]
            "Audit <- ShipmentStarted",            // 11
            "Audit fire log record",               // 12
            "Audit <- InvoiceSettled",             // 13
            "Audit fire log record",               // 14
            "log ready -> ready",                  // 15
            "log ready -> ready",                  // 16
        ])
    );
    assert_eq!(
        causes(&trace),
        [
            None,
            Some(0),
            Some(1),
            Some(2),
            Some(3),
            Some(2),
            Some(5),
            Some(4),
            Some(7),
            Some(6),
            Some(9),
            Some(8),
            Some(11),
            Some(10),
            Some(13),
            Some(12),
            Some(14)
        ]
    );
    assert_eq!(finals(&model, &trace), ["o1: paid", "s1: picking", "i1: settled", "log: ready"]);
    assert_eq!(trace.scenario, "paid");
    assert_eq!(trace.ordering, None);
}

#[test]
fn causes_always_point_backwards() {
    let model = model(CASCADE);
    let trace = run(&model, PAID);
    for (i, step) in trace.steps.iter().enumerate() {
        if let Some(cause) = step.cause {
            assert!(cause.index() < i, "step {i} caused by later step {}", cause.index());
        }
    }
}

const ORDERS: &str = r#"
machines:
  Order:
    initial: draft
    fields: [orderId]
    states: [draft, pending, paid, cancelled]
    transitions:
      - { from: draft,   to: pending,   on: submit }
      - { from: pending, to: paid,      on: capture_ok, emits: [OrderPaid] }
      - { from: pending, to: cancelled, on: timeout,    emits: [OrderCancelled] }
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
          target: Shipment where orderId == event.orderId
external:
  Customer: [Order.submit]
  PaymentGateway: [Order.capture_ok]
  Clock: [Order.timeout]
"#;

fn clock_scenario(timing: &str) -> String {
    format!(
        r#"scenario: clock
instances:
  o1: {{ machine: Order, fields: {{ orderId: 1 }} }}
  s1: {{ machine: Shipment, fields: {{ orderId: 1 }} }}
steps:
  - {{ source: Customer, fire: Order.submit, target: o1 }}
  - {{ source: PaymentGateway, fire: Order.capture_ok, target: o1 }}
  - {{ source: Clock, fire: Order.timeout, target: o1, timing: {timing} }}
"#
    )
}

#[test]
fn after_quiescence_steps_wait_for_the_cascade() {
    let model = model(ORDERS);
    let trace = run(&model, &clock_scenario("after-quiescence"));
    assert_eq!(
        lines(&model, &trace),
        strs(&[
            "ext Customer -> o1 submit",
            "o1 draft -> pending",
            "ext PaymentGateway -> o1 capture_ok",
            "o1 pending -> paid",
            "o1 emit OrderPaid",
            "Fulfillment <- OrderPaid",
            "Fulfillment fire s1 start",
            "s1 idle -> picking",
            "ext Clock -> o1 timeout",
            "o1 drop timeout @ paid",
        ])
    );
}

#[test]
fn immediate_steps_interleave_with_the_running_cascade() {
    let model = model(ORDERS);
    let trace = run(&model, &clock_scenario("immediate"));
    assert_eq!(
        lines(&model, &trace),
        strs(&[
            "ext Customer -> o1 submit",
            "o1 draft -> pending",
            "ext PaymentGateway -> o1 capture_ok",
            "o1 pending -> paid",
            "o1 emit OrderPaid",
            "ext Clock -> o1 timeout",
            "o1 drop timeout @ paid",
            "Fulfillment <- OrderPaid",
            "Fulfillment fire s1 start",
            "s1 idle -> picking",
        ])
    );
    assert_eq!(causes(&trace)[5], None);
    assert_eq!(causes(&trace)[6], Some(5));
    assert_eq!(causes(&trace)[7], Some(4));
}

#[test]
fn immediate_steps_let_two_cascades_interleave() {
    let model = model(ORDERS);
    let trace = run(
        &model,
        r#"scenario: two orders
instances:
  o1: { machine: Order, fields: { orderId: 1 }, state: pending }
  o2: { machine: Order, fields: { orderId: 2 }, state: pending }
  s1: { machine: Shipment, fields: { orderId: 1 } }
  s2: { machine: Shipment, fields: { orderId: 2 } }
steps:
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
  - { source: PaymentGateway, fire: Order.capture_ok, target: o2, timing: immediate }
"#,
    );
    assert_eq!(
        lines(&model, &trace),
        strs(&[
            "ext PaymentGateway -> o1 capture_ok",
            "o1 pending -> paid",
            "o1 emit OrderPaid",
            "ext PaymentGateway -> o2 capture_ok",
            "o2 pending -> paid",
            "o2 emit OrderPaid",
            "Fulfillment <- OrderPaid",
            "Fulfillment fire s1 start",
            "Fulfillment <- OrderPaid",
            "Fulfillment fire s2 start",
            "s1 idle -> picking",
            "s2 idle -> picking",
        ])
    );
}

#[test]
fn triggers_without_an_enabled_transition_are_dropped() {
    let model = model(ORDERS);
    let trace = run(
        &model,
        r#"scenario: drops
instances:
  o1: { machine: Order, fields: { orderId: 1 } }
  s1: { machine: Shipment, fields: { orderId: 1 }, state: picking }
steps:
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
  - { source: Customer, fire: Order.submit, target: o1 }
  - { source: Customer, fire: Order.submit, target: o1 }
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1 }
"#,
    );
    assert_eq!(
        lines(&model, &trace),
        strs(&[
            "ext PaymentGateway -> o1 capture_ok",
            "o1 drop capture_ok @ draft",
            "ext Customer -> o1 submit",
            "o1 draft -> pending",
            "ext Customer -> o1 submit",
            "o1 drop submit @ pending",
            "ext PaymentGateway -> o1 capture_ok",
            "o1 pending -> paid",
            "o1 emit OrderPaid",
            "Fulfillment <- OrderPaid",
            "Fulfillment fire s1 start",
            // Controller fires are dropped the same way.
            "s1 drop start @ picking",
        ])
    );
    assert!(matches!(trace.steps[11].kind, TraceStepKind::Dropped { .. }));
    assert_eq!(trace.steps[11].cause, Some(StepIx(10)));
}

#[test]
fn the_first_transition_in_definition_order_wins_over_guards() {
    let model = model(
        r#"
machines:
  Payment:
    states: [new, approved, review]
    transitions:
      - { from: new, to: review,   on: check, guard: "amount > 1000" }
      - { from: new, to: approved, on: check, guard: "amount <= 1000" }
external:
  Api: [Payment.check]
"#,
    );
    let trace =
        run(&model, "scenario: g\ninstances:\n  p: Payment\nsteps:\n  - { source: Api, fire: Payment.check }\n");
    assert_eq!(lines(&model, &trace)[1], "p new -> review");
}

#[test]
fn transitions_on_ancestors_apply_to_nested_states() {
    let model = model(
        r#"
machines:
  Job:
    initial: running
    states:
      - running:
          states: [fetching, parsing]
      - failed
    transitions:
      - { from: running.fetching, to: running.parsing, on: fetched }
      - { from: running, to: failed, on: crash }
external:
  Os: [Job.fetched, Job.crash]
"#,
    );
    let trace = run(
        &model,
        "scenario: j\ninstances:\n  j: Job\nsteps:\n  - { source: Os, fire: Job.fetched }\n  - { source: Os, fire: Job.crash }\n",
    );
    assert_eq!(
        lines(&model, &trace),
        strs(&[
            "ext Os -> j fetched",
            "j running.fetching -> running.parsing",
            "ext Os -> j crash",
            "j running.parsing -> failed"
        ])
    );
}

#[test]
fn payloads_are_fields_overlaid_with_the_trigger_payload() {
    let model = model(
        r#"
machines:
  Order:
    initial: pending
    states: [pending, paid]
    transitions:
      - { from: pending, to: paid, on: capture_ok, emits: [OrderPaid] }
  Shipment:
    states: [idle, picking]
    transitions:
      - { from: idle, to: picking, on: start, emits: [ShipmentStarted] }
controllers:
  Fulfillment:
    on:
      OrderPaid:
        - fire: Shipment.start
          target: Shipment where orderId == event.orderId
external:
  PaymentGateway: [Order.capture_ok]
"#,
    );
    let run = match simulate_run(
        &model,
        &scenario(
            r#"scenario: payloads
instances:
  o1: { machine: Order, fields: { orderId: 1, region: eu } }
  s1: { machine: Shipment, fields: { orderId: 1, carrier: ups } }
steps:
  - { source: PaymentGateway, fire: Order.capture_ok, target: o1, payload: { amount: 42, region: us } }
"#,
        ),
    ) {
        Ok(run) => run,
        Err(err) => panic!("{err}"),
    };
    let text = |ix: u32| {
        run.payloads.get(&StepIx(ix)).map(|p| p.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" "))
    };
    assert_eq!(
        lines(&model, &run.trace),
        strs(&[
            "ext PaymentGateway -> o1 capture_ok",
            "o1 pending -> paid",
            "o1 emit OrderPaid",
            "Fulfillment <- OrderPaid",
            "Fulfillment fire s1 start",
            "s1 idle -> picking",
            "s1 emit ShipmentStarted",
        ])
    );
    // The step's payload travels with the external fire.
    assert_eq!(text(0).as_deref(), Some("amount=42 region=us"));
    // The step payload wins over the instance's fields.
    assert_eq!(text(2).as_deref(), Some("amount=42 orderId=1 region=us"));
    // A fired transition's events carry its instance's fields overlaid with
    // the handled event's payload.
    assert_eq!(text(6).as_deref(), Some("amount=42 carrier=ups orderId=1 region=us"));
    // Only external fires and emits carry payloads.
    assert_eq!(run.payloads.keys().map(|k| k.0).collect::<Vec<_>>(), [0, 2, 6]);
}

#[test]
fn steps_without_payload_record_none() {
    let model = model(ORDERS);
    let run = match simulate_run(&model, &scenario(&clock_scenario("immediate"))) {
        Ok(run) => run,
        Err(err) => panic!("{err}"),
    };
    assert!(!run.payloads.contains_key(&StepIx(0)));
    assert!(run.payloads.contains_key(&StepIx(4)));
}

#[test]
fn an_unbounded_cycle_hits_the_step_limit() {
    let model = model(
        r#"
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
"#,
    );
    let err =
        run_err(&model, "scenario: forever\ninstances:\n  p: Ping\nsteps:\n  - { source: Starter, fire: Ping.go }\n");
    assert_eq!(err, SimError::StepLimit(STEP_LIMIT));
    assert!(err.to_string().contains("unbounded cycle"));
}

#[test]
fn a_long_but_bounded_cascade_is_fine() {
    let model = model(
        r#"
machines:
  Counter:
    states: [c0, c1, c2, c3, c4, c5]
    transitions:
      - { from: c0, to: c1, on: inc, emits: [Bumped] }
      - { from: c1, to: c2, on: inc, emits: [Bumped] }
      - { from: c2, to: c3, on: inc, emits: [Bumped] }
      - { from: c3, to: c4, on: inc, emits: [Bumped] }
      - { from: c4, to: c5, on: inc, emits: [Bumped] }
controllers:
  Again:
    on:
      Bumped: { fire: Counter.inc }
external:
  Starter: [Counter.inc]
"#,
    );
    let trace =
        run(&model, "scenario: count\ninstances:\n  c: Counter\nsteps:\n  - { source: Starter, fire: Counter.inc }\n");
    assert_eq!(finals(&model, &trace), ["c: c5"]);
    assert_eq!(lines(&model, &trace).last().map(String::as_str), Some("c drop inc @ c5"));
}

#[test]
fn runs_are_deterministic() {
    let model: Model = model(CASCADE);
    let a = simulate(&model, &scenario(PAID));
    let b = simulate(&model, &scenario(PAID));
    assert_eq!(a, b);
}
