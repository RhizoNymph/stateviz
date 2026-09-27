//! Scenario file parsing: the accepted shapes, every error kind and spans.

mod common;

use cascade_core::error::Expected;
use cascade_core::span::Pos;
use cascade_sim::{ScenarioErrorKind, StepTiming, ValueMap};
use common::{has, scenario, scenario_err};

const FULL: &str = r#"scenario: happy path
instances:
  o1: { machine: Order, fields: { orderId: "1" } }
  s1:
    machine: Shipment
    fields:
      orderId: 1
      priority: true
    state: idle
  log: Log
steps:
  - { source: Customer, fire: Order.submit, target: o1 }
  - source: PaymentGateway
    fire: Order.capture_ok
    target: o1
    payload: { amount: 42, currency: EUR }
  - { source: Clock, fire: Order.timeout, target: o1, timing: immediate }
  - { source: Clock, fire: Log.tick, timing: after-quiescence }
  - { source: Clock, fire: Log.tick, timing: after_quiescence }
"#;

#[test]
fn parses_a_full_scenario() {
    let s = scenario(FULL);
    assert_eq!(s.name, "happy path");

    let names: Vec<&str> = s.instances.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["o1", "s1", "log"]);
    let machines: Vec<&str> = s.instances.iter().map(|i| i.machine.as_str()).collect();
    assert_eq!(machines, ["Order", "Shipment", "Log"]);

    let o1 = &s.instances[0];
    assert_eq!(o1.fields.get("orderId"), Some("1"));
    assert_eq!(o1.state, None);

    // Numbers and booleans are read as text.
    let s1 = &s.instances[1];
    assert_eq!(s1.fields.get("orderId"), Some("1"));
    assert_eq!(s1.fields.get("priority"), Some("true"));
    assert_eq!(s1.state.as_ref().map(|st| st.as_str()), Some("idle"));

    // `log: Log` is shorthand for an instance with no fields.
    assert!(s.instances[2].fields.is_empty());
    assert_eq!(s.instances[2].state, None);

    assert_eq!(s.steps.len(), 5);
    let first = &s.steps[0];
    assert_eq!(first.source.as_str(), "Customer");
    assert_eq!(first.fire.value.machine, "Order");
    assert_eq!(first.fire.value.trigger, "submit");
    assert_eq!(first.target.as_ref().map(|t| t.as_str()), Some("o1"));
    assert!(first.payload.is_empty());
    assert_eq!(first.timing, StepTiming::AfterQuiescence);

    let second = &s.steps[1];
    assert_eq!(
        second.payload.to_payload().into_iter().collect::<Vec<_>>(),
        [("amount".to_owned(), "42".to_owned()), ("currency".to_owned(), "EUR".to_owned())]
    );

    assert_eq!(s.steps[2].timing, StepTiming::Immediate);
    assert_eq!(s.steps[3].target, None);
    assert_eq!(s.steps[3].timing, StepTiming::AfterQuiescence);
    assert_eq!(s.steps[4].timing, StepTiming::AfterQuiescence);
}

#[test]
fn records_spans() {
    let s = scenario(FULL);
    let o1 = &s.instances[0];
    assert_eq!(o1.name.span.start.line, 3);
    assert_eq!(o1.machine.span.start.line, 3);
    let s1 = &s.instances[1];
    assert_eq!(s1.name.span.start.line, 4);
    assert_eq!(s1.state.as_ref().map(|st| st.span.start.line), Some(9));
    assert_eq!(s1.fields.entry("priority").map(|e| e.key_span.start.line), Some(8));
    assert_eq!(s1.fields.entry("priority").map(|e| e.value.span.start.line), Some(8));
    // An instance's span runs from its name to the end of its body.
    assert_eq!(s1.span.start.line, 4);
    assert!(s1.span.end.line >= 9);

    let lines: Vec<u32> = s.steps.iter().map(|st| st.span.start.line).collect();
    assert_eq!(lines, [12, 13, 17, 18, 19]);
    assert_eq!(s.steps[1].fire.span.start.line, 14);
    assert_eq!(s.steps[1].target.as_ref().map(|t| t.span.start.line), Some(15));
    // Columns point at the value, after the key.
    let fire = s.steps[0].fire.span.start;
    let source = s.steps[0].source.span.start;
    assert_eq!(fire.line, source.line);
    assert!(fire.col > source.col);
}

#[test]
fn instances_and_payload_may_be_omitted_or_null() {
    let s = scenario("scenario: empty\nsteps: []\n");
    assert!(s.instances.is_empty());
    assert!(s.steps.is_empty());

    let s = scenario("scenario: nulls\ninstances: ~\nsteps:\n  - { source: A, fire: M.t, target: x, payload: ~ }\n");
    assert!(s.instances.is_empty());
    assert!(s.steps[0].payload.is_empty());

    let s = scenario("scenario: null steps\nsteps: ~\n");
    assert!(s.steps.is_empty());
}

#[test]
fn value_maps_built_in_code_have_no_positions() {
    let map: ValueMap = [("orderId", "1")].into_iter().collect();
    assert_eq!(map.get("orderId"), Some("1"));
    assert!(!map.entry("orderId").is_some_and(|e| e.key_span.is_known()));
}

// --- Errors -------------------------------------------------------------------

#[test]
fn yaml_syntax_error_has_a_position() {
    let err = scenario_err("scenario: x\nsteps: [\n");
    assert_eq!(err.diagnostics.len(), 1);
    assert!(matches!(err.diagnostics[0].kind, ScenarioErrorKind::YamlSyntax { .. }));
    assert!(err.diagnostics[0].span.is_known());
}

#[test]
fn empty_and_multi_document_files() {
    for text in ["", "# only a comment\n", "~\n"] {
        let err = scenario_err(text);
        assert!(has(&err, |k| matches!(k, ScenarioErrorKind::EmptyDocument)), "{text:?}");
    }
    let err = scenario_err("scenario: a\nsteps: []\n---\nscenario: b\nsteps: []\n");
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::MultipleDocuments)));
    // The second document starts on the line after `---`.
    assert_eq!(err.diagnostics[0].span.start.line, 4);
}

#[test]
fn aliases_are_expanded() {
    let s = scenario("scenario: a\ninstances:\n  o1: &o { machine: Order }\n  o2: *o\nsteps: []\n");
    let machines: Vec<&str> = s.instances.iter().map(|i| i.machine.as_str()).collect();
    assert_eq!(machines, ["Order", "Order"]);
}

#[test]
fn wrong_types() {
    let cases: &[(&str, Expected)] = &[
        ("- just a list\n", Expected::Mapping),
        ("scenario: a\nsteps: {}\n", Expected::Sequence),
        ("scenario: a\ninstances: [o1]\nsteps: []\n", Expected::Mapping),
        ("scenario: a\ninstances:\n  o1: [Order]\nsteps: []\n", Expected::Mapping),
        ("scenario: a\nsteps:\n  - just a string\n", Expected::Mapping),
        ("scenario: [a]\nsteps: []\n", Expected::String),
        ("scenario: a\nsteps:\n  - { source: A, fire: M.t, payload: [1, 2] }\n", Expected::Mapping),
        ("scenario: a\nsteps:\n  - { source: A, fire: M.t, payload: { x: { y: 1 } } }\n", Expected::String),
        ("scenario: a\ninstances:\n  o1: { machine: M, fields: { x: ~ } }\nsteps: []\n", Expected::String),
    ];
    for (text, expected) in cases {
        let err = scenario_err(text);
        assert!(
            has(&err, |k| matches!(k, ScenarioErrorKind::WrongType { expected: e, .. } if e == expected)),
            "{text:?}: {err}"
        );
    }
}

#[test]
fn unknown_keys_are_reported_where_they_are() {
    let err = scenario_err(
        "scenario: a\ndescription: nope\ninstances:\n  o1: { machine: M, colour: red }\nsteps:\n  - { source: A, fire: M.t, when: later }\n",
    );
    let keys: Vec<(String, u32)> = err
        .diagnostics
        .iter()
        .filter_map(|d| match &d.kind {
            ScenarioErrorKind::UnknownKey { key, .. } => Some((key.clone(), d.span.start.line)),
            _ => None,
        })
        .collect();
    assert_eq!(keys, [("description".to_owned(), 2), ("colour".to_owned(), 4), ("when".to_owned(), 6)]);
}

#[test]
fn missing_keys() {
    let cases: &[(&str, &str)] = &[
        ("steps: []\n", "scenario"),
        ("scenario: a\n", "steps"),
        ("scenario: a\ninstances:\n  o1: { fields: { x: 1 } }\nsteps: []\n", "machine"),
        ("scenario: a\nsteps:\n  - { fire: M.t }\n", "source"),
        ("scenario: a\nsteps:\n  - { source: A }\n", "fire"),
    ];
    for (text, key) in cases {
        let err = scenario_err(text);
        assert!(
            has(&err, |k| matches!(k, ScenarioErrorKind::MissingKey { key: missing, .. } if missing == key)),
            "{text:?}: {err}"
        );
    }
}

#[test]
fn invalid_names() {
    let cases: &[(&str, &str)] = &[
        ("scenario: a\ninstances:\n  1st: { machine: M }\nsteps: []\n", "1st"),
        ("scenario: a\ninstances:\n  o1: { machine: 'Order form' }\nsteps: []\n", "Order form"),
        ("scenario: a\ninstances:\n  o1: 'Order form'\nsteps: []\n", "Order form"),
        ("scenario: a\ninstances:\n  o1: { machine: M, fields: { 'order id': 1 } }\nsteps: []\n", "order id"),
        ("scenario: a\ninstances:\n  o1: { machine: M, state: 'a..b' }\nsteps: []\n", "a..b"),
        ("scenario: a\nsteps:\n  - { source: 'Payment Gateway', fire: M.t }\n", "Payment Gateway"),
        ("scenario: a\nsteps:\n  - { source: A, fire: M.t, target: 'o 1' }\n", "o 1"),
        ("scenario: a\nsteps:\n  - { source: A, fire: M.t, payload: { '-x': 1 } }\n", "-x"),
    ];
    for (text, bad) in cases {
        let err = scenario_err(text);
        assert!(
            has(&err, |k| matches!(k, ScenarioErrorKind::InvalidName { name, .. } if name == bad)),
            "{text:?}: {err}"
        );
    }
}

#[test]
fn invalid_trigger_refs() {
    for fire in ["Order", "Order.submit.now", ".submit", "'Order. submit'"] {
        let text = format!("scenario: a\nsteps:\n  - {{ source: A, fire: {fire} }}\n");
        let err = scenario_err(&text);
        assert!(has(&err, |k| matches!(k, ScenarioErrorKind::InvalidTriggerRef { .. })), "{fire}: {err}");
    }
}

#[test]
fn unknown_timing() {
    let err = scenario_err("scenario: a\nsteps:\n  - { source: A, fire: M.t, timing: later }\n");
    assert!(has(&err, |k| matches!(k, ScenarioErrorKind::UnknownTiming { text } if text == "later")));
    assert_eq!(err.diagnostics[0].span.start.line, 3);
}

#[test]
fn every_problem_is_reported_in_source_order() {
    let err = scenario_err(
        r#"scenario: many problems
instances:
  o1: { machine: Order, extra: 1 }
  2x: { machine: Order }
steps:
  - { source: Customer, fire: Order }
  - { source: Clock, fire: Order.timeout, timing: soon }
  - { fire: Order.submit }
"#,
    );
    assert_eq!(err.diagnostics.len(), 5, "{err}");
    let lines: Vec<u32> = err.diagnostics.iter().map(|d| d.span.start.line).collect();
    assert_eq!(lines, [3, 4, 6, 7, 8]);
    let mut sorted = err.diagnostics.iter().map(|d| d.span).collect::<Vec<_>>();
    sorted.sort();
    assert_eq!(sorted, err.diagnostics.iter().map(|d| d.span).collect::<Vec<_>>());
}

#[test]
fn display_includes_positions() {
    let err = scenario_err("scenario: a\nsteps:\n  - { source: A, fire: M.t, timing: later }\n");
    let text = err.to_string();
    let Pos { line, col } = err.diagnostics[0].span.start;
    assert!(text.starts_with(&format!("{line}:{col}: ")), "{text}");
    assert!(text.contains("unknown timing `later`"), "{text}");
}
