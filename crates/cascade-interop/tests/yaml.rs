//! The YAML writer: compact style, minimal quoting, and round trips that
//! preserve every element and attribute.

mod common;

use cascade_core::definition::{
    Definition, EventDef, MachineDef, StateDef, StateKindDef, TargetMode, TargetSpec, TransitionDef, ValueExpr,
};
use cascade_core::{Spanned, load_str, parse_definition};
use cascade_interop::{ExportFormat, export, to_yaml};

use common::{assert_yaml_round_trip, example_definitions, load};

const KITCHEN_SINK: &str = r#"
system: Kitchen Sink
machines:
  Order:
    color: blue
    domain: sales
    initial: draft
    fields: [orderId, customer]
    states:
      - draft
      - pending:
          initial: authorizing
          states:
            - waiting
            - authorizing
            - hist: { kind: history }
            - deep: { kind: deep-history }
      - paid: { kind: final }
      - cancelled
    transitions:
      - { from: draft, to: pending, on: submit, guard: "amount > 0", emits: [OrderSubmitted] }
      - { from: [draft, pending.waiting], to: cancelled, on: cancel }
      - { from: pending, to: paid, on: capture_ok, emits: [OrderPaid, Audit], bounded: true }
      - { from: pending, to: paid, on: capture_ok, guard: "retry: #2" }
      - { from: pending.authorizing, to: pending.hist, on: interrupt }
  Shipment:
    color: sky-blue
    fields: [orderId, carrier]
    states: [idle, picking, shipped]
    transitions:
      - { from: idle, to: picking, on: start }
      - { from: picking, to: shipped, on: handoff, emits: [Shipped] }
events:
  OrderSubmitted: {}
  OrderPaid: { payload: [orderId, amount] }
  Audit: {}
  Shipped: { payload: [orderId] }
controllers:
  Fulfillment:
    on:
      OrderPaid:
        - fire: Shipment.start
          target: Shipment where orderId == event.orderId
          when: not a gift card
        - fire: Shipment.start
          target: all Shipment where orderId == "A, B"
          bounded: true
        - fire: Shipment.start
          target: new Shipment with orderId = event.orderId, carrier = 'the "fast" one'
      Audit: []
  Idle:
    on: {}
external:
  Customer: [Order.submit, Order.cancel]
  Gateway: [Order.capture_ok]
  Nobody: []
"#;

#[test]
fn every_example_round_trips() {
    for (path, text) in example_definitions() {
        let model = load(&text);
        let reloaded = assert_yaml_round_trip(&model);
        assert_eq!(reloaded.transition_count(), model.transition_count(), "{}", path.display());
    }
}

#[test]
fn kitchen_sink_round_trips() {
    let model = load(KITCHEN_SINK);
    assert_yaml_round_trip(&model);
}

#[test]
fn export_yaml_is_to_yaml_of_the_definition() {
    let model = load(KITCHEN_SINK);
    let exported = export(ExportFormat::Yaml, &model).expect("yaml export");
    assert_eq!(exported, to_yaml(model.definition()));
}

#[test]
fn spec_example_keeps_the_compact_aligned_style() {
    let model = load(&common::read("examples/order-fulfillment/cascade.yaml"));
    let yaml = to_yaml(model.definition());
    let expected = "\
machines:
  Order:
    color: blue
    initial: draft
    states: [draft, pending, paid, cancelled]
    transitions:
      - { from: draft,   to: pending,   on: submit }
      - { from: pending, to: paid,      on: capture_ok, emits: [OrderPaid] }
      - { from: pending, to: cancelled, on: timeout,    emits: [OrderCancelled] }

  Shipment:
    color: green
    initial: idle
    states: [idle, picking, shipped]
    transitions:
      - { from: idle,    to: picking, on: start }
      - { from: picking, to: shipped, on: handoff, emits: [Shipped] }

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
";
    assert_eq!(yaml, expected);
}

#[test]
fn nested_states_use_block_lists_and_one_line_kinds() {
    let model = load(KITCHEN_SINK);
    let yaml = to_yaml(model.definition());
    assert!(yaml.contains("    states:\n      - draft\n      - pending:\n          initial: authorizing\n"), "{yaml}");
    assert!(yaml.contains("            - hist: { kind: history }\n"), "{yaml}");
    assert!(yaml.contains("            - deep: { kind: deep-history }\n"), "{yaml}");
    assert!(yaml.contains("      - paid: { kind: final }\n"), "{yaml}");
    // Siblings without bodies stay a flow list.
    assert!(yaml.contains("    states: [idle, picking, shipped]\n"), "{yaml}");
}

#[test]
fn multi_source_from_lists_are_kept() {
    let model = load(KITCHEN_SINK);
    let yaml = to_yaml(model.definition());
    assert!(yaml.contains("from: [draft, pending.waiting],"), "{yaml}");
}

#[test]
fn quotes_only_when_needed() {
    let model = load(KITCHEN_SINK);
    let yaml = to_yaml(model.definition());
    assert!(yaml.contains("system: Kitchen Sink\n"), "{yaml}");
    assert!(yaml.contains("guard: amount > 0,"), "{yaml}");
    assert!(yaml.contains(r#"guard: "retry: #2" }"#), "{yaml}");
    assert!(yaml.contains("when: not a gift card\n"), "{yaml}");
    assert!(yaml.contains("target: Shipment where orderId == event.orderId\n"), "{yaml}");
    // Block context allows commas; the selector's own quotes survive.
    assert!(yaml.contains(r#"target: all Shipment where orderId == "A, B""#), "{yaml}");
    assert!(yaml.contains("  Audit: []\n") || yaml.contains("      Audit: []\n"), "{yaml}");
    assert!(yaml.contains("  Nobody: []\n"), "{yaml}");
    assert!(yaml.contains("    on: {}\n"), "{yaml}");
}

fn state(name: &str) -> StateDef {
    StateDef {
        name: Spanned::synthetic(name.to_owned()),
        kind: Spanned::synthetic(StateKindDef::Normal),
        initial: None,
        states: Vec::new(),
        span: Default::default(),
    }
}

fn machine(name: &str, states: Vec<StateDef>, transitions: Vec<TransitionDef>) -> MachineDef {
    MachineDef {
        name: Spanned::synthetic(name.to_owned()),
        color: None,
        domain: None,
        initial: None,
        fields: Vec::new(),
        states,
        transitions,
        span: Default::default(),
    }
}

fn transition(from: &str, to: &str, on: &str, guard: Option<&str>) -> TransitionDef {
    TransitionDef {
        from: vec![Spanned::synthetic(from.to_owned())],
        to: Spanned::synthetic(to.to_owned()),
        on: Spanned::synthetic(on.to_owned()),
        guard: guard.map(|g| Spanned::synthetic(g.to_owned())),
        emits: Vec::new(),
        bounded: false,
        span: Default::default(),
    }
}

/// Guards and other free text survive whatever characters they contain.
#[test]
fn tricky_free_text_round_trips() {
    let guards = [
        "true",
        "false",
        "null",
        "~",
        "yes",
        "no",
        "on",
        "off",
        "y",
        "n",
        "Yes",
        "NO",
        "1",
        "1.0",
        "1e3",
        "0x1F",
        "0o17",
        ".5",
        "-1",
        "+1",
        ".inf",
        "-.inf",
        ".nan",
        "1_000",
        "1:30",
        "2001-12-14",
        "0b101",
        "",
        " ",
        "  padded  ",
        "a: b",
        "a:b",
        "a #b",
        "a#b",
        "#x",
        "- x",
        "-x",
        "? x",
        "[x]",
        "{x}",
        "x, y",
        "&anchor",
        "*alias",
        "!tag",
        "|",
        ">",
        "'single'",
        "\"double\"",
        "it's",
        "say \"hi\"",
        "back\\slash",
        "tab\there",
        "new\nline",
        "cr\rreturn",
        "bell\u{7}",
        "unicode ✓ café",
        "emoji 🚀",
        "\u{2028}sep",
        "%percent",
        "@at",
        "`tick`",
        "=",
        "<<",
        "a > b",
        "a >= 1 && b <= 2",
        "x || y",
        "{a: 1}",
        "[1, 2]",
        "trailing:",
        "http://example.com/x?y=1#z",
    ];
    let transitions: Vec<TransitionDef> =
        guards.iter().enumerate().map(|(i, g)| transition("a", "b", &format!("t{i}"), Some(g))).collect();
    let definition = Definition {
        system: Some(Spanned::synthetic("Weird: #system, [1]".to_owned())),
        machines: vec![machine("M", vec![state("a"), state("b")], transitions)],
        ..Definition::default()
    };
    let yaml = to_yaml(&definition);
    let parsed = match parse_definition(&yaml) {
        Ok(d) => d,
        Err(err) => panic!("{err}\n---\n{yaml}"),
    };
    assert_eq!(parsed.system.map(|s| s.value).as_deref(), Some("Weird: #system, [1]"));
    let back: Vec<Option<String>> =
        parsed.machines[0].transitions.iter().map(|t| t.guard.as_ref().map(|g| g.value.clone())).collect();
    let expected: Vec<Option<String>> = guards.iter().map(|g| Some((*g).to_owned())).collect();
    assert_eq!(back, expected, "\n{yaml}");
}

/// Names that YAML would read as booleans, nulls or numbers are quoted in
/// every position a name appears.
#[test]
fn reserved_word_names_round_trip() {
    let text = r#"
machines:
  "true":
    initial: "null"
    states: ["null", "false", "on", "yes"]
    transitions:
      - { from: "null", to: "false", on: "true", emits: ["null"] }
      - { from: "on", to: "yes", on: "no" }
controllers:
  "false":
    on:
      "null":
        - fire: true.no
external:
  "null": [true.true]
"#;
    let model = load(text);
    let reloaded = assert_yaml_round_trip(&model);
    assert!(reloaded.machine_by_name("true").is_some());
}

#[test]
fn empty_definition_emits_an_empty_machines_map() {
    let yaml = to_yaml(&Definition::default());
    assert_eq!(yaml, "machines: {}\n");
    assert!(load_str(&yaml).is_ok());
}

#[test]
fn selector_literals_are_quoted_when_the_tokenizer_needs_it() {
    let spec = |value: &str| TargetSpec {
        mode: TargetMode::One,
        machine: "Shipment".to_owned(),
        clauses: vec![cascade_core::definition::FieldClause {
            field: "orderId".to_owned(),
            value: ValueExpr::Literal(value.to_owned()),
        }],
    };
    for literal in ["plain", "event.orderId", "two words", "", "quote\"d", "a,b", "x==y"] {
        let text = "machines:\n  Shipment:\n    fields: [orderId]\n    states: [idle]\n    transitions:\n      - { from: idle, to: idle, on: go }\ncontrollers:\n  C:\n    on:\n      E:\n        - fire: Shipment.go\n          target: PLACEHOLDER\n";
        let model = load(&text.replace("PLACEHOLDER", "Shipment"));
        // Swap the placeholder target for one with the tricky literal.
        let mut definition = model.definition().clone();
        definition.controllers[0].on[0].rules[0].target = Some(Spanned::synthetic(spec(literal)));
        let yaml = to_yaml(&definition);
        let parsed = match parse_definition(&yaml) {
            Ok(d) => d,
            Err(err) => panic!("{literal:?}: {err}\n---\n{yaml}"),
        };
        let target = parsed.controllers[0].on[0].rules[0].target.as_ref().map(|t| t.value.clone());
        assert_eq!(target, Some(spec(literal)), "{literal:?}\n{yaml}");
    }
}

#[test]
fn declared_events_keep_strict_mode() {
    let definition = Definition {
        machines: vec![machine("M", vec![state("a")], vec![])],
        events: vec![EventDef {
            name: Spanned::synthetic("Unused".to_owned()),
            payload: vec![Spanned::synthetic("id".to_owned())],
            span: Default::default(),
        }],
        ..Definition::default()
    };
    let yaml = to_yaml(&definition);
    assert!(yaml.contains("events:\n  Unused: { payload: [id] }\n"), "{yaml}");
    let model = load(&yaml);
    assert!(model.event(model.event_by_name("Unused").expect("event")).declared);
}
