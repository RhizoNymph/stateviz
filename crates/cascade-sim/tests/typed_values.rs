//! Scenario values are checked against declared field and payload types
//! (`ScenarioErrorKind::InvalidValue`).

mod common;
mod support;

use cascade_sim::{ScenarioErrorKind, SimError, validate};
use common::{model, scenario};
use support::{add, apply_err, fire_with, session};

const TYPED: &str = r#"
events:
  Paid: { payload: { amount: number, method: "enum [card, cash]" } }
machines:
  Order:
    fields: { orderId: string, amount: number, gift: bool, tier: "enum [basic, gold]", note: any }
    initial: open
    states: [open, paid]
    transitions:
      - { from: open, to: paid, on: pay, emits: [Paid] }
external:
  Api: [Order.pay]
"#;

fn invalid(text: &str) -> Vec<(String, String, u32, u32)> {
    let model = model(TYPED);
    match validate(&model, &scenario(text)) {
        Ok(_) => Vec::new(),
        Err(err) => err
            .diagnostics
            .iter()
            .filter_map(|d| match &d.kind {
                ScenarioErrorKind::InvalidValue { field, expected, .. } => {
                    Some((field.clone(), expected.clone(), d.span.start.line, d.span.start.col))
                }
                _ => None,
            })
            .collect(),
    }
}

fn with_fields(fields: &str) -> String {
    format!("scenario: t\ninstances:\n  o: {{ machine: Order, fields: {{ {fields} }} }}\nsteps: []\n")
}

#[test]
fn instance_fields_must_read_as_their_types() {
    assert_eq!(invalid(&with_fields("amount: 12.5, gift: true, tier: gold, orderId: x, note: whatever")), []);
    assert_eq!(invalid(&with_fields("amount: lots")), [("amount".into(), "a decimal number".into(), 3, 42)]);
    let found = invalid(&with_fields("gift: yes, tier: platinum"));
    let fields: Vec<&str> = found.iter().map(|(f, ..)| f.as_str()).collect();
    assert_eq!(fields, ["gift", "tier"]);
    assert_eq!(found[1].1, "one of basic, gold");
}

#[test]
fn created_instances_are_checked_too() {
    let text = "scenario: t\nsteps:\n  - { create: o, machine: Order, fields: { amount: lots } }\n";
    assert_eq!(invalid(text).len(), 1);
}

#[test]
fn step_payloads_are_checked_against_fields_and_emitted_payloads() {
    let base = "scenario: t\ninstances:\n  o: Order\nsteps:\n  - { source: Api, fire: Order.pay, payload: { ";
    assert_eq!(invalid(&format!("{base}amount: 5, method: cash }} }}\n")), []);
    // `amount` is a number field of the fired machine.
    assert_eq!(invalid(&format!("{base}amount: five }} }}\n")).len(), 1);
    // `method` is declared on the emitted event's payload.
    let found = invalid(&format!("{base}method: cheque }} }}\n"));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, "method");
    // Undeclared keys accept anything.
    assert_eq!(invalid(&format!("{base}extra: whatever }} }}\n")), []);
}

#[test]
fn the_message_names_the_value_and_the_expectation() {
    let model = model(TYPED);
    let err = match validate(&model, &scenario(&with_fields("amount: lots"))) {
        Ok(_) => panic!("expected a diagnostic"),
        Err(err) => err,
    };
    assert_eq!(err.to_string(), "3:42: `lots` is not a valid `amount`: expected a decimal number");
}

#[test]
fn play_actions_are_checked_too() {
    let model = model(TYPED);
    let mut s = session(&model, &[]);
    let err = apply_err(&mut s, &model, add("o", "Order", &[("amount", "lots")]));
    assert!(matches!(err, SimError::Action(ScenarioErrorKind::InvalidValue { .. })), "{err}");
    let mut s = session(&model, &[add("o", "Order", &[])]);
    let err = apply_err(&mut s, &model, fire_with("Api", "Order.pay", "o", &[("method", "cheque")]));
    assert!(matches!(err, SimError::Action(ScenarioErrorKind::InvalidValue { .. })), "{err}");
}
