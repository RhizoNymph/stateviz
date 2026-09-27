//! The definition schema, walked over marked YAML.

use saphyr::{LoadableYamlNode, MarkedYamlOwned};

use crate::color::PaletteColor;
use crate::definition::{
    ControllerDef, Definition, EventDef, ExternalDef, HandlerDef, MachineDef, RuleDef, StateDef, StateKindDef,
    TransitionDef,
};
use crate::error::{DiagnosticKind, Expected, LoadError};
use crate::parse::grammar::{parse_target, parse_trigger_ref};
use crate::parse::node::{Diags, as_mapping, as_sequence, is_null, pos_of, span_of};
use crate::span::{SourceSpan, Spanned};

pub fn parse(text: &str) -> Result<Definition, LoadError> {
    let documents = match MarkedYamlOwned::load_from_str(text) {
        Ok(docs) => docs,
        Err(err) => {
            let pos = pos_of(*err.marker());
            return Err(LoadError::single(
                DiagnosticKind::YamlSyntax { message: err.info().to_owned() },
                SourceSpan::new(pos, pos),
            ));
        }
    };

    let root = match documents.as_slice() {
        [] => return Err(LoadError::single(DiagnosticKind::EmptyDocument, SourceSpan::unknown())),
        [root] if is_null(root) => {
            return Err(LoadError::single(DiagnosticKind::EmptyDocument, span_of(root)));
        }
        [root] => root,
        [_, second, ..] => {
            return Err(LoadError::single(DiagnosticKind::MultipleDocuments, span_of(second)));
        }
    };

    let mut diags = Diags::default();
    let definition = parse_root(root, &mut diags);
    if diags.list.is_empty() {
        Ok(definition)
    } else {
        diags.list.sort_by_key(|d| d.span);
        Err(LoadError { diagnostics: diags.list })
    }
}

fn parse_root(root: &MarkedYamlOwned, diags: &mut Diags) -> Definition {
    let mut definition = Definition::default();
    let Some(mapping) = diags.mapping(root, "definition") else {
        return definition;
    };
    let fields = diags.fields(mapping, "definition", &["system", "machines", "events", "controllers", "external"]);

    definition.system = fields.get("system").and_then(|n| diags.string(n, "system"));

    if let Some(node) = fields.require("machines", diags, "definition", span_of(root))
        && let Some(machines) = diags.mapping(node, "machines")
    {
        for (name, body) in diags.named_entries(machines, "machines") {
            if let Some(machine) = parse_machine(name, body, diags) {
                definition.machines.push(machine);
            }
        }
    }

    if let Some(node) = fields.get("events") {
        definition.events = parse_events(node, diags);
    }

    if let Some(node) = fields.get("controllers")
        && !is_null(node)
        && let Some(controllers) = diags.mapping(node, "controllers")
    {
        for (name, body) in diags.named_entries(controllers, "controllers") {
            if let Some(controller) = parse_controller(name, body, diags) {
                definition.controllers.push(controller);
            }
        }
    }

    if let Some(node) = fields.get("external")
        && !is_null(node)
        && let Some(sources) = diags.mapping(node, "external")
    {
        for (name, body) in diags.named_entries(sources, "external") {
            let context = format!("external source `{}`", name.value);
            let triggers =
                diags.string_or_list(body, &context).into_iter().filter_map(|text| trigger_ref(text, diags)).collect();
            definition.external.push(ExternalDef { span: name.span, name, triggers });
        }
    }

    definition
}

fn trigger_ref(text: Spanned<String>, diags: &mut Diags) -> Option<Spanned<crate::definition::TriggerRef>> {
    match parse_trigger_ref(&text.value) {
        Some(parsed) => Some(Spanned::new(parsed, text.span)),
        None => {
            diags.push(DiagnosticKind::InvalidTriggerRef { text: text.value }, text.span);
            None
        }
    }
}

fn parse_machine(name: Spanned<String>, body: &MarkedYamlOwned, diags: &mut Diags) -> Option<MachineDef> {
    let context = format!("machine `{}`", name.value);
    let mapping = diags.mapping(body, &context)?;
    let fields = diags.fields(mapping, &context, &["color", "domain", "initial", "fields", "states", "transitions"]);

    let color = fields.get("color").and_then(|node| {
        let text = diags.string(node, &format!("{context} color"))?;
        match text.value.parse::<PaletteColor>() {
            Ok(color) => Some(Spanned::new(color, text.span)),
            Err(err) => {
                diags.push(err.into(), text.span);
                None
            }
        }
    });
    let domain = fields.get("domain").and_then(|n| diags.string(n, &format!("{context} domain")));
    let initial = fields.get("initial").and_then(|n| diags.path(n, &format!("{context} initial")));
    let machine_fields =
        fields.get("fields").map(|n| diags.name_or_list(n, &format!("{context} fields"))).unwrap_or_default();

    let states = match fields.require("states", diags, &context, name.span) {
        Some(node) => parse_states(node, &context, diags),
        None => Vec::new(),
    };

    let mut transitions = Vec::new();
    if let Some(node) = fields.get("transitions")
        && !is_null(node)
    {
        match as_sequence(node) {
            Some(items) => {
                for item in items {
                    if let Some(t) = parse_transition(item, &context, diags) {
                        transitions.push(t);
                    }
                }
            }
            None => diags.wrong_type(node, &format!("{context} transitions"), Expected::Sequence),
        }
    }

    Some(MachineDef { name, color, domain, initial, fields: machine_fields, states, transitions, span: span_of(body) })
}

/// `states:` is either a sequence whose items are names or single-key
/// mappings `{name: body}`, or a mapping `name: body`.
fn parse_states(node: &MarkedYamlOwned, context: &str, diags: &mut Diags) -> Vec<StateDef> {
    let states_context = format!("{context} states");
    let mut states = Vec::new();
    if let Some(items) = as_sequence(node) {
        for item in items {
            if let Some(mapping) = as_mapping(item) {
                for (name, body) in diags.named_entries(mapping, &states_context) {
                    if let Some(state) = parse_state(name, Some(body), context, diags) {
                        states.push(state);
                    }
                }
            } else if let Some(name) = diags.name(item, &states_context)
                && let Some(state) = parse_state(name, None, context, diags)
            {
                states.push(state);
            }
        }
    } else if let Some(mapping) = as_mapping(node) {
        for (name, body) in diags.named_entries(mapping, &states_context) {
            if let Some(state) = parse_state(name, Some(body), context, diags) {
                states.push(state);
            }
        }
    } else {
        diags.wrong_type(node, &states_context, Expected::SequenceOrMapping);
    }
    states
}

fn parse_state(
    name: Spanned<String>,
    body: Option<&MarkedYamlOwned>,
    parent_context: &str,
    diags: &mut Diags,
) -> Option<StateDef> {
    let default_kind = Spanned::new(StateKindDef::Normal, name.span);
    let Some(body) = body.filter(|b| !is_null(b)) else {
        return Some(StateDef { span: name.span, name, kind: default_kind, initial: None, states: Vec::new() });
    };

    let context = format!("{parent_context} state `{}`", name.value);
    let mapping = diags.mapping(body, &context)?;
    let fields = diags.fields(mapping, &context, &["kind", "initial", "states"]);

    let kind = match fields.get("kind").and_then(|n| diags.string(n, &format!("{context} kind"))) {
        None => default_kind,
        Some(text) => match text.value.as_str() {
            "normal" => Spanned::new(StateKindDef::Normal, text.span),
            "final" => Spanned::new(StateKindDef::Final, text.span),
            "history" => Spanned::new(StateKindDef::History, text.span),
            "deep-history" | "deep_history" => Spanned::new(StateKindDef::DeepHistory, text.span),
            _ => {
                diags.push(DiagnosticKind::UnknownStateKind(text.value), text.span);
                default_kind
            }
        },
    };
    let initial = fields.get("initial").and_then(|n| diags.name(n, &format!("{context} initial")));
    let states =
        fields.get("states").filter(|n| !is_null(n)).map(|n| parse_states(n, &context, diags)).unwrap_or_default();

    Some(StateDef { span: name.span, name, kind, initial, states })
}

fn parse_transition(item: &MarkedYamlOwned, context: &str, diags: &mut Diags) -> Option<TransitionDef> {
    let context = format!("{context} transition");
    let span = span_of(item);
    let mapping = diags.mapping(item, &context)?;
    let fields = diags.fields(mapping, &context, &["from", "to", "on", "guard", "emits", "bounded"]);

    let from = fields.require("from", diags, &context, span).map(|n| diags.path_or_list(n, &format!("{context} from")));
    let to = fields.require("to", diags, &context, span).and_then(|n| diags.path(n, &format!("{context} to")));
    let on = fields.require("on", diags, &context, span).and_then(|n| diags.name(n, &format!("{context} trigger")));
    let guard = fields.get("guard").and_then(|n| diags.string(n, &format!("{context} guard")));
    let emits = fields
        .get("emits")
        .filter(|n| !is_null(n))
        .map(|n| diags.name_or_list(n, &format!("{context} emits")))
        .unwrap_or_default();
    let bounded = fields.get("bounded").and_then(|n| diags.bool(n, &format!("{context} bounded"))).unwrap_or(false);

    let from = from.filter(|f| !f.is_empty())?;
    Some(TransitionDef { from, to: to?, on: on?, guard, emits, bounded, span })
}

fn parse_events(node: &MarkedYamlOwned, diags: &mut Diags) -> Vec<EventDef> {
    let mut events = Vec::new();
    if is_null(node) {
        return events;
    }
    if let Some(items) = as_sequence(node) {
        for item in items {
            if let Some(name) = diags.name(item, "events") {
                events.push(EventDef { span: name.span, name, payload: Vec::new() });
            }
        }
    } else if let Some(mapping) = as_mapping(node) {
        for (name, body) in diags.named_entries(mapping, "events") {
            let context = format!("event `{}`", name.value);
            let mut payload = Vec::new();
            if !is_null(body)
                && let Some(body_map) = diags.mapping(body, &context)
            {
                let fields = diags.fields(body_map, &context, &["payload"]);
                if let Some(p) = fields.get("payload").filter(|n| !is_null(n)) {
                    payload = diags.name_or_list(p, &format!("{context} payload"));
                }
            }
            events.push(EventDef { span: name.span, name, payload });
        }
    } else {
        diags.wrong_type(node, "events", Expected::SequenceOrMapping);
    }
    events
}

fn parse_controller(name: Spanned<String>, body: &MarkedYamlOwned, diags: &mut Diags) -> Option<ControllerDef> {
    let context = format!("controller `{}`", name.value);
    let mapping = diags.mapping(body, &context)?;
    let fields = diags.fields(mapping, &context, &["on"]);
    let mut handlers = Vec::new();
    if let Some(on) = fields.require("on", diags, &context, name.span)
        && let Some(subscriptions) = diags.mapping(on, &format!("{context} on"))
    {
        for (event, rules_node) in diags.named_entries(subscriptions, &format!("{context} on")) {
            let handler_context = format!("{context} handler for `{}`", event.value);
            let rule_nodes: Vec<&MarkedYamlOwned> = if let Some(items) = as_sequence(rules_node) {
                items.iter().collect()
            } else if as_mapping(rules_node).is_some() {
                vec![rules_node]
            } else {
                diags.wrong_type(rules_node, &handler_context, Expected::SequenceOrMapping);
                Vec::new()
            };
            let rules = rule_nodes.into_iter().filter_map(|r| parse_rule(r, &handler_context, diags)).collect();
            handlers.push(HandlerDef { span: event.span, event, rules });
        }
    }
    Some(ControllerDef { span: name.span, name, on: handlers })
}

fn parse_rule(node: &MarkedYamlOwned, context: &str, diags: &mut Diags) -> Option<RuleDef> {
    let context = format!("{context} rule");
    let span = span_of(node);
    let mapping = diags.mapping(node, &context)?;
    let fields = diags.fields(mapping, &context, &["fire", "target", "when", "bounded"]);

    let fire = fields
        .require("fire", diags, &context, span)
        .and_then(|n| diags.string(n, &format!("{context} fire")))
        .and_then(|text| trigger_ref(text, diags));
    let target = fields.get("target").and_then(|n| diags.string(n, &format!("{context} target"))).and_then(|text| {
        match parse_target(&text.value) {
            Ok(spec) => Some(Spanned::new(spec, text.span)),
            Err(reason) => {
                diags.push(DiagnosticKind::InvalidSelector { text: text.value, reason }, text.span);
                None
            }
        }
    });
    let when = fields.get("when").and_then(|n| diags.string(n, &format!("{context} when")));
    let bounded = fields.get("bounded").and_then(|n| diags.bool(n, &format!("{context} bounded"))).unwrap_or(false);

    Some(RuleDef { fire: fire?, target, when, bounded, span })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definition::{TargetMode, TriggerRef};
    use crate::span::Pos;

    const SPEC_EXAMPLE: &str = r#"
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
"#;

    fn kinds(err: &LoadError) -> Vec<&DiagnosticKind> {
        err.diagnostics.iter().map(|d| &d.kind).collect()
    }

    #[test]
    fn parses_the_spec_example() {
        let def = parse(SPEC_EXAMPLE).expect("spec example parses");
        assert_eq!(def.machines.len(), 2);
        let order = &def.machines[0];
        assert_eq!(order.name.value, "Order");
        assert_eq!(order.name.span.start.line, 3);
        assert_eq!(order.color.as_ref().map(|c| c.value), Some(PaletteColor::Blue));
        assert_eq!(order.initial.as_ref().map(|i| i.as_str()), Some("draft"));
        let names: Vec<_> = order.states.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["draft", "pending", "paid", "cancelled"]);
        assert_eq!(order.transitions.len(), 3);
        let paid = &order.transitions[1];
        assert_eq!(paid.on.value, "capture_ok");
        assert_eq!(paid.emits[0].value, "OrderPaid");
        assert_eq!(paid.span.start.line, 9);

        let rule = &def.controllers[0].on[0].rules[0];
        assert_eq!(rule.fire.value, TriggerRef { machine: "Shipment".into(), trigger: "start".into() });
        let target = rule.target.as_ref().expect("target");
        assert_eq!(target.value.mode, TargetMode::One);

        let externals: Vec<_> = def.external.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(externals, ["Customer", "PaymentGateway", "Clock"]);
    }

    #[test]
    fn parses_nested_states_in_both_forms() {
        let text = r#"
machines:
  Job:
    states:
      queued: {}
      running:
        initial: fetching
        states:
          - fetching
          - computing: { kind: normal }
          - hist: { kind: deep-history }
      done: { kind: final }
    transitions: []
"#;
        let def = parse(text).expect("parses");
        let job = &def.machines[0];
        assert_eq!(job.states.len(), 3);
        let running = &job.states[1];
        assert_eq!(running.initial.as_ref().map(|s| s.as_str()), Some("fetching"));
        let children: Vec<_> = running.states.iter().map(|s| (s.name.as_str(), s.kind.value)).collect();
        assert_eq!(
            children,
            [
                ("fetching", StateKindDef::Normal),
                ("computing", StateKindDef::Normal),
                ("hist", StateKindDef::DeepHistory)
            ]
        );
        assert_eq!(job.states[2].kind.value, StateKindDef::Final);
    }

    #[test]
    fn single_rule_mapping_and_scalar_external() {
        let text = r#"
machines:
  A: { states: [x, y], transitions: [{ from: [x, y], to: y, on: go, guard: n > 0, bounded: true }] }
controllers:
  C:
    on:
      E: { fire: A.go, when: "flag set" }
external:
  User: A.go
"#;
        let def = parse(text).expect("parses");
        let t = &def.machines[0].transitions[0];
        assert_eq!(t.from.len(), 2);
        assert_eq!(t.guard.as_ref().map(|g| g.as_str()), Some("n > 0"));
        assert!(t.bounded);
        assert_eq!(def.controllers[0].on[0].rules.len(), 1);
        assert_eq!(def.external[0].triggers.len(), 1);
    }

    #[test]
    fn collects_every_shape_error() {
        let text = r#"
machines:
  Order:
    colour: blue
    states: [draft, "bad name"]
    transitions:
      - { from: draft, on: submit }
      - { from: draft, to: draft, on: x, emits: [Ok], bounded: maybe }
controllers:
  C:
    on:
      E:
        - fire: NoDot
        - fire: A.b
          target: "Shipment where"
external:
  User: [Order.submit]
"#;
        let err = parse(text).expect_err("has errors");
        let found = kinds(&err);
        assert!(found.iter().any(|k| matches!(k, DiagnosticKind::UnknownKey { key, .. } if key == "colour")));
        assert!(found.iter().any(|k| matches!(k, DiagnosticKind::InvalidName { name, .. } if name == "bad name")));
        assert!(found.iter().any(|k| matches!(k, DiagnosticKind::MissingKey { key, .. } if key == "to")));
        assert!(found.iter().any(|k| matches!(k, DiagnosticKind::WrongType { expected: Expected::Bool, .. })));
        assert!(found.iter().any(|k| matches!(k, DiagnosticKind::InvalidTriggerRef { text } if text == "NoDot")));
        assert!(found.iter().any(|k| matches!(k, DiagnosticKind::InvalidSelector { .. })));
        // Diagnostics come back in source order.
        let lines: Vec<_> = err.diagnostics.iter().map(|d| d.span.start.line).collect();
        let mut sorted = lines.clone();
        sorted.sort_unstable();
        assert_eq!(lines, sorted);
    }

    #[test]
    fn columns_are_one_based() {
        // `y` is the 26th character of line 4.
        let text = "machines:\n  A:\n    states: [x]\n    transitions: [{from: y, to: x, on: go}]\n";
        let def = parse(text).expect("parses");
        let from = &def.machines[0].transitions[0].from[0];
        assert_eq!(from.span.start, Pos::new(4, 26));
        // `A` is the 3rd character of line 2.
        assert_eq!(def.machines[0].name.span.start, Pos::new(2, 3));
    }

    #[test]
    fn reports_yaml_syntax_errors_with_position() {
        let err = parse("machines:\n  A: [unclosed\n").expect_err("syntax error");
        assert!(matches!(err.diagnostics[0].kind, DiagnosticKind::YamlSyntax { .. }));
        assert!(err.diagnostics[0].span.is_known());
    }

    #[test]
    fn rejects_empty_and_multi_document_files() {
        assert!(matches!(kinds(&parse("").expect_err("empty"))[0], DiagnosticKind::EmptyDocument));
        assert!(matches!(kinds(&parse("# only a comment\n").expect_err("empty"))[0], DiagnosticKind::EmptyDocument));
        assert!(matches!(
            kinds(&parse("machines: {}\n---\nmachines: {}\n").expect_err("two docs"))[0],
            DiagnosticKind::MultipleDocuments
        ));
    }

    #[test]
    fn requires_machines_key() {
        let err = parse("controllers: {}\n").expect_err("no machines");
        assert!(matches!(kinds(&err)[0], DiagnosticKind::MissingKey { key, .. } if key == "machines"));
    }

    #[test]
    fn unknown_state_kind_and_color() {
        let text = "machines:\n  A:\n    color: chartreuse\n    states:\n      x: { kind: weird }\n";
        let err = parse(text).expect_err("errors");
        let found = kinds(&err);
        assert!(found.iter().any(|k| matches!(k, DiagnosticKind::UnknownColor(_))));
        assert!(found.iter().any(|k| matches!(k, DiagnosticKind::UnknownStateKind(s) if s == "weird")));
    }
}
