//! XState v5 import of the semantics-phase features: numeric `after`
//! delays as Cascade timers, guards as choices (`?name`, `?type` for
//! parameterized guards), and `assign` actions as `sets` when they copy an
//! event field.

mod common;

use cascade_interop::{ImportFormat, Imported, import, to_yaml};

fn imported(json: &str) -> Imported {
    match import(ImportFormat::XState, json) {
        Ok(i) => i,
        Err(err) => panic!("{err}"),
    }
}

#[test]
fn numeric_after_delays_are_timers() {
    let json = r#"{ "id": "session", "initial": "active", "states": {
        "active": { "after": { "30000": "expired", "1500": { "target": "warned" } }, "on": { "PING": "active" } },
        "warned": { "after": { "90000": "expired" } },
        "expired": { "type": "final" } } }"#;
    let i = imported(json);
    let yaml = to_yaml(&i.definition);
    assert!(yaml.contains("after: 30s"), "{yaml}");
    assert!(yaml.contains("after: 1500ms"), "{yaml}");
    assert!(yaml.contains("after: 90s"), "{yaml}");
    assert!(!yaml.contains("Clock"), "{yaml}");
    common::resolve_ok(i.definition);
}

#[test]
fn guards_are_choices_named_by_their_type() {
    let json = r#"{ "id": "m", "states": {
        "a": { "on": { "GO": [
            { "target": "b", "guard": "isAdmin" },
            { "target": "b", "guard": { "type": "above", "params": { "min": 1 } } },
            { "target": "b", "guard": { "type": "xstate.not", "guards": ["isAdmin"] } } ] } },
        "b": {} } }"#;
    let yaml = to_yaml(&imported(json).definition);
    assert!(yaml.contains(r#"guard: "?isAdmin""#), "{yaml}");
    assert!(yaml.contains(r#"guard: "?above""#), "{yaml}");
    assert!(yaml.contains(r#"guard: "?!(isAdmin)""#), "{yaml}");
}

#[test]
fn assigns_copying_event_fields_become_sets() {
    let json = r#"{ "id": "order", "states": {
        "draft": { "on": { "PAY": { "target": "paid", "actions": [
            { "type": "xstate.assign", "assignment": { "amount": "({ event }) => event.amount", "orderId": "event.id" } },
            { "type": "assign", "params": { "assignment": { "count": "({ context }) => context.count + 1" } } },
            "assign",
            "emit:Paid" ] } } },
        "paid": { "entry": [ { "type": "xstate.assign", "assignment": { "x": "event.x" } } ] } } }"#;
    let i = imported(json);
    let yaml = to_yaml(&i.definition);
    assert!(yaml.contains("sets: { amount: event.amount, orderId: event.id }"), "{yaml}");
    assert!(yaml.contains("emits: [Paid]"), "{yaml}");
    let messages: Vec<String> = i.warnings.iter().map(ToString::to_string).collect();
    assert!(messages.iter().any(|m| m.contains("assignment of `count`")), "{messages:?}");
    assert!(messages.iter().any(|m| m.contains("without a static assignment")), "{messages:?}");
    assert!(messages.iter().any(|m| m.contains("entry or exit")), "{messages:?}");
    common::resolve_ok(i.definition);
}
