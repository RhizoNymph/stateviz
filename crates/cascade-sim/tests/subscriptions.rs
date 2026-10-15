//! Machines that subscribe to events: consumer order, `Notify` steps,
//! `where` filters, ignored deliveries and `consumers_left`.

mod common;
mod support;

use cascade_sim::PendingKind;
use common::{causes, lines, model, run, strs};
use support::{add, apply, fire, remove, session, step};

const SAGA: &str = r#"
machines:
  Order:
    fields: [orderId]
    initial: draft
    states: [draft, placed]
    transitions:
      - { from: draft, to: placed, on: place, emits: [OrderPlaced] }
  Checkout:
    fields: [orderId]
    initial: waiting
    states: [waiting, started]
    transitions:
      - from: waiting
        to: started
        on: { event: OrderPlaced, where: "orderId == event.orderId" }
        emits: [CheckoutStarted]
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

/// Instances declared in an order different from the model's machine order.
const PLACE: &str = r#"scenario: place
instances:
  a1: Audit
  r1: Receipt
  c2: { machine: Checkout, fields: { orderId: "2" } }
  c1: { machine: Checkout, fields: { orderId: "1" } }
  o1: { machine: Order, fields: { orderId: "1" } }
steps:
  - { source: Customer, fire: Order.place, target: o1 }
"#;

#[test]
fn subscribers_come_first_in_model_then_creation_order_then_controllers() {
    let model = model(SAGA);
    let trace = run(&model, PLACE);
    assert_eq!(
        lines(&model, &trace),
        strs(&[
            "ext Customer -> o1 place",
            "o1 draft -> placed",
            "o1 emit OrderPlaced",
            // c2 (created first) does not match `where` and records nothing.
            "c1 notify OrderPlaced",
            "c1 waiting -> started",
            "c1 emit CheckoutStarted",
            "a1 notify OrderPlaced",
            "a1 idle -> logged",
            "Mailer <- OrderPlaced",
            "Mailer fire r1 send",
            "r1 idle -> sent",
        ])
    );
    // Notify is caused by the emit, the transition by the notify.
    assert_eq!(causes(&trace)[3..8], [Some(2), Some(3), Some(4), Some(2), Some(6)]);
    assert_eq!(common::finals(&model, &trace), ["o1: placed", "c2: waiting", "c1: started", "a1: logged", "r1: sent"]);
}

#[test]
fn an_instance_that_accepts_nothing_ignores_the_event() {
    let model = model(SAGA);
    let text = "scenario: twice\ninstances:\n  a1: Audit\n  o1: Order\n  o2: Order\nsteps:\n  - { source: Customer, fire: Order.place, target: o1 }\n  - { source: Customer, fire: Order.place, target: o2 }\n";
    let trace = run(&model, text);
    let notifies = lines(&model, &trace).iter().filter(|l| l.contains("notify")).count();
    // The second OrderPlaced finds a1 already logged: nothing recorded.
    assert_eq!(notifies, 1);
}

#[test]
fn consumers_left_lists_subscribing_instances_first() {
    let model = model(SAGA);
    let s = support::from_scenario(&model, &common::scenario(&PLACE.replace("steps:\n", "end: pause\nsteps:\n")));
    let pending = s.pending();
    assert_eq!(pending.len(), 1);
    assert!(matches!(pending[0].kind, PendingKind::Event { .. }));
    // Every live instance of a subscribing machine is a consumer, matching
    // `where` or not.
    assert_eq!(pending[0].consumers_left, strs(&["c2", "c1", "a1", "Mailer"]));
}

#[test]
fn consumers_are_fixed_when_the_event_is_queued() {
    let model = model(SAGA);
    let mut s = session(&model, &[add("o1", "Order", &[("orderId", "1")]), add("a1", "Audit", &[])]);
    apply(&mut s, &model, fire("Customer", "Order.place", "o1"));
    // Created after the emit: not a consumer of it.
    apply(&mut s, &model, add("a2", "Audit", &[]));
    assert_eq!(s.pending()[0].consumers_left, strs(&["a1", "Mailer"]));
    // Removed before delivery: no longer a consumer.
    apply(&mut s, &model, remove("a1"));
    assert_eq!(s.pending()[0].consumers_left, strs(&["Mailer"]));
    apply(&mut s, &model, step());
    let states: Vec<String> =
        s.instances().iter().map(|i| format!("{} {}", i.name, model.state(i.state).path)).collect();
    assert_eq!(states, strs(&["o1 placed", "a2 idle"]));
}

#[test]
fn a_false_guard_on_a_subscription_ignores_the_event() {
    let model = model(
        r#"
machines:
  Order:
    initial: open
    states: [open, paid]
    transitions:
      - { from: open, to: paid, on: pay, emits: [Paid] }
  Watcher:
    initial: idle
    states: [idle, big]
    transitions:
      - { from: idle, to: big, on: { event: Paid }, guard: "event.amount > 100" }
external:
  Api: [Order.pay]
"#,
    );
    let trace = run(
        &model,
        "scenario: s\ninstances:\n  o: Order\n  w: Watcher\nsteps:\n  - { source: Api, fire: Order.pay, payload: { amount: 5 } }\n",
    );
    assert_eq!(lines(&model, &trace), strs(&["ext Api -> o pay", "o open -> paid", "o emit Paid"]));
}

#[test]
fn an_event_nobody_handles_is_still_delivered_to_subscribers() {
    let model = model(
        r#"
machines:
  A:
    initial: x
    states: [x, y]
    transitions:
      - { from: x, to: y, on: go, emits: [Went] }
  B:
    initial: x
    states: [x, y]
    transitions:
      - { from: x, to: y, on: { event: Went }, emits: [Went] }
external:
  Api: [A.go]
"#,
    );
    // B's own `Went` reaches B again, which ignores it (already in y).
    let trace = run(&model, "scenario: s\ninstances:\n  a: A\n  b: B\nsteps:\n  - { source: Api, fire: A.go }\n");
    assert_eq!(
        lines(&model, &trace),
        strs(&["ext Api -> a go", "a x -> y", "a emit Went", "b notify Went", "b x -> y", "b emit Went"])
    );
}

#[test]
fn subscription_steps_have_text() {
    let model = model(SAGA);
    let trace = run(&model, PLACE);
    let text = cascade_sim::step_text(&model, &trace, &trace.steps[3], None);
    assert_eq!(text, "c1 receives OrderPlaced");
}
