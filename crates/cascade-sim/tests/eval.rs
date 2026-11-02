//! Guard and condition evaluation: which candidate a trigger takes, with
//! the right scope and declared types in each context.

mod common;

use cascade_sim::TraceStepKind;
use common::{lines, model, run, strs};

const PAY: &str = r#"
machines:
  Order:
    fields: { amount: number, tier: "enum [basic, gold]" }
    initial: pending
    states: [pending, gold, big, small]
    transitions:
      - { from: pending, to: gold,  on: pay, guard: "self.tier == gold" }
      - { from: pending, to: big,   on: pay, guard: "event.amount >= 100" }
      - { from: pending, to: small, on: pay, guard: else }
external:
  Customer: [Order.pay]
"#;

fn pay(tier: &str, payload: &str) -> String {
    let model = model(PAY);
    let scenario = format!(
        "scenario: p\ninstances:\n  o: {{ machine: Order, fields: {{ tier: {tier} }} }}\nsteps:\n  - {{ source: Customer, fire: Order.pay{payload} }}\n"
    );
    let trace = run(&model, &scenario);
    lines(&model, &trace)[1..].join(" | ")
}

#[test]
fn a_self_field_guard_reads_the_instance() {
    assert_eq!(pay("gold", ", payload: { amount: 5 }"), "o pending -> gold");
}

#[test]
fn an_event_guard_reads_the_triggers_payload() {
    assert_eq!(pay("basic", ", payload: { amount: 150 }"), "o pending -> big");
}

#[test]
fn else_is_taken_when_nothing_else_holds() {
    assert_eq!(pay("basic", ", payload: { amount: 5 }"), "o pending -> small");
    // A missing payload value makes the test false.
    assert_eq!(pay("basic", ""), "o pending -> small");
}

#[test]
fn else_declared_first_still_waits_for_the_others() {
    let model = model(
        r#"
machines:
  Order:
    initial: pending
    states: [pending, big, small]
    transitions:
      - { from: pending, to: small, on: pay, guard: else }
      - { from: pending, to: big,   on: pay, guard: "event.amount >= 100" }
external:
  Customer: [Order.pay]
"#,
    );
    let scenario = |amount: &str| {
        format!(
            "scenario: p\ninstances:\n  o: Order\nsteps:\n  - {{ source: Customer, fire: Order.pay, payload: {{ amount: {amount} }} }}\n"
        )
    };
    assert_eq!(lines(&model, &run(&model, &scenario("150")))[1], "o pending -> big");
    assert_eq!(lines(&model, &run(&model, &scenario("15")))[1], "o pending -> small");
}

#[test]
fn several_holding_expressions_take_the_first_without_a_choice() {
    let model = model(
        r#"
machines:
  Order:
    initial: pending
    states: [pending, a, b]
    transitions:
      - { from: pending, to: a, on: pay, guard: "event.amount > 1" }
      - { from: pending, to: b, on: pay, guard: "event.amount > 2" }
external:
  Customer: [Order.pay]
"#,
    );
    let trace = run(
        &model,
        "scenario: p\ninstances:\n  o: Order\nsteps:\n  - { source: Customer, fire: Order.pay, payload: { amount: 5 } }\n",
    );
    assert_eq!(lines(&model, &trace), strs(&["ext Customer -> o pay", "o pending -> a"]));
}

#[test]
fn no_holding_guard_drops_the_trigger() {
    let model = model(
        r#"
machines:
  Order:
    initial: pending
    states: [pending, big]
    transitions:
      - { from: pending, to: big, on: pay, guard: "event.amount >= 100" }
external:
  Customer: [Order.pay]
"#,
    );
    let trace = run(
        &model,
        "scenario: p\ninstances:\n  o: Order\nsteps:\n  - { source: Customer, fire: Order.pay, payload: { amount: 5 } }\n",
    );
    assert!(matches!(trace.steps[1].kind, TraceStepKind::Dropped { .. }));
}

#[test]
fn declared_types_decide_how_values_compare() {
    // `self.n` is a number: 10 > 9 (as text "10" < "9"). An enum value that
    // is not a member never matches.
    let model = model(
        r#"
machines:
  Item:
    fields: { n: number, tier: "enum [basic, gold]" }
    initial: idle
    states: [idle, num, gold, none]
    transitions:
      - { from: idle, to: gold, on: go, guard: "self.tier == gold" }
      - { from: idle, to: num,  on: go, guard: "self.n > event.n" }
      - { from: idle, to: none, on: go, guard: else }
external:
  Api: [Item.go]
"#,
    );
    let go = |fields: &str, payload: &str| {
        let scenario = format!(
            "scenario: t\ninstances:\n  i: {{ machine: Item, fields: {{ {fields} }} }}\nsteps:\n  - {{ source: Api, fire: Item.go, payload: {{ {payload} }} }}\n"
        );
        lines(&model, &run(&model, &scenario))[1].clone()
    };
    assert_eq!(go("n: 10, tier: basic", "n: 9"), "i idle -> num");
    assert_eq!(go("n: 9, tier: basic", "n: 10"), "i idle -> none");
}

#[test]
fn subscription_guards_read_the_events_declared_payload() {
    // `event.amount` is declared a number on the subscribed event, so "10" >
    // "9" compares numerically even though the instance declares nothing.
    let model = model(
        r#"
events:
  Paid: { payload: { amount: number } }
machines:
  Order:
    initial: open
    states: [open, paid]
    transitions:
      - { from: open, to: paid, on: pay, emits: [Paid] }
  Watcher:
    initial: idle
    states: [idle, big, small]
    transitions:
      - { from: idle, to: big, on: { event: Paid }, guard: "event.amount > 9" }
      - { from: idle, to: small, on: { event: Paid }, guard: else }
external:
  Api: [Order.pay]
"#,
    );
    let scenario = |amount: &str| {
        format!(
            "scenario: s\ninstances:\n  o: Order\n  w: Watcher\nsteps:\n  - {{ source: Api, fire: Order.pay, payload: {{ amount: {amount} }} }}\n"
        )
    };
    let big = lines(&model, &run(&model, &scenario("10")));
    assert!(big.contains(&"w idle -> big".to_owned()), "{big:#?}");
    let small = lines(&model, &run(&model, &scenario("8")));
    assert!(small.contains(&"w idle -> small".to_owned()), "{small:#?}");
}

#[test]
fn controller_fires_carry_the_handled_events_payload_to_guards() {
    let model = model(
        r#"
machines:
  Order:
    initial: open
    states: [open, paid]
    transitions:
      - { from: open, to: paid, on: pay, emits: [Paid] }
  Ledger:
    initial: idle
    states: [idle, big, small]
    transitions:
      - { from: idle, to: big, on: book, guard: "event.amount >= 100" }
      - { from: idle, to: small, on: book, guard: else }
controllers:
  Books:
    on:
      Paid: { fire: Ledger.book, target: Ledger }
external:
  Api: [Order.pay]
"#,
    );
    let scenario = |amount: &str| {
        format!(
            "scenario: c\ninstances:\n  o: Order\n  l: Ledger\nsteps:\n  - {{ source: Api, fire: Order.pay, payload: {{ amount: {amount} }} }}\n"
        )
    };
    assert!(lines(&model, &run(&model, &scenario("500"))).contains(&"l idle -> big".to_owned()));
    assert!(lines(&model, &run(&model, &scenario("5"))).contains(&"l idle -> small".to_owned()));
}

#[test]
fn rule_conditions_read_the_handled_events_declared_types() {
    let model = model(
        r#"
events:
  Paid: { payload: { amount: number } }
machines:
  Order:
    initial: open
    states: [open, paid]
    transitions:
      - { from: open, to: paid, on: pay, emits: [Paid] }
  Ledger:
    initial: idle
    states: [idle, big]
    transitions:
      - { from: idle, to: big, on: big }
controllers:
  Books:
    on:
      Paid: { fire: Ledger.big, target: Ledger, when: "event.amount > 9" }
external:
  Api: [Order.pay]
"#,
    );
    let scenario = |amount: &str| {
        format!(
            "scenario: c\ninstances:\n  o: Order\n  l: Ledger\nsteps:\n  - {{ source: Api, fire: Order.pay, payload: {{ amount: {amount} }} }}\n"
        )
    };
    // Numerically 10 > 9 (as text "10" < "9").
    assert!(lines(&model, &run(&model, &scenario("10"))).contains(&"l idle -> big".to_owned()));
    assert!(!lines(&model, &run(&model, &scenario("8"))).contains(&"l idle -> big".to_owned()));
}
