//! Lookups on a [`Definition`] for building edit ops: names in use, states
//! by path, handlers and rules by key, and synthetic (span-less) elements.

use std::collections::BTreeSet;

use cascade_core::definition::{
    ControllerDef, ExternalDef, HandlerDef, MachineDef, RuleDef, StateDef, StateKindDef, TransitionDef, TriggerRef,
};
use cascade_core::{Definition, SourceSpan, Spanned};

pub fn synthetic(text: &str) -> Spanned<String> {
    Spanned::synthetic(text.to_owned())
}

pub fn state_def(name: &str) -> StateDef {
    StateDef {
        name: synthetic(name),
        kind: Spanned::synthetic(StateKindDef::Normal),
        initial: None,
        states: Vec::new(),
        span: SourceSpan::unknown(),
    }
}

/// A machine with one state, the smallest that resolves.
pub fn machine_def(name: &str, first_state: &str) -> MachineDef {
    MachineDef {
        name: synthetic(name),
        color: None,
        domain: None,
        initial: None,
        fields: Vec::new(),
        states: vec![state_def(first_state)],
        transitions: Vec::new(),
        span: SourceSpan::unknown(),
    }
}

pub fn controller_def(name: &str) -> ControllerDef {
    ControllerDef { name: synthetic(name), on: Vec::new(), span: SourceSpan::unknown() }
}

pub fn external_def(name: &str) -> ExternalDef {
    ExternalDef { name: synthetic(name), triggers: Vec::new(), span: SourceSpan::unknown() }
}

pub fn handler_def(event: &str) -> HandlerDef {
    HandlerDef { event: synthetic(event), rules: Vec::new(), span: SourceSpan::unknown() }
}

/// A rule firing `fire` at the one instance of its machine.
pub fn rule_def(fire: TriggerRef) -> RuleDef {
    RuleDef { fire: Spanned::synthetic(fire), target: None, when: None, bounded: false, span: SourceSpan::unknown() }
}

pub fn transition_def(from: &str, to: &str, trigger: &str) -> TransitionDef {
    TransitionDef {
        from: vec![synthetic(from)],
        to: synthetic(to),
        on: synthetic(trigger),
        guard: None,
        emits: Vec::new(),
        bounded: false,
        span: SourceSpan::unknown(),
    }
}

pub fn machine<'a>(definition: &'a Definition, name: &str) -> Option<&'a MachineDef> {
    definition.machines.iter().find(|m| m.name.value == name)
}

pub fn controller<'a>(definition: &'a Definition, name: &str) -> Option<&'a ControllerDef> {
    definition.controllers.iter().find(|c| c.name.value == name)
}

pub fn external<'a>(definition: &'a Definition, name: &str) -> Option<&'a ExternalDef> {
    definition.external.iter().find(|e| e.name.value == name)
}

pub fn handler<'a>(definition: &'a Definition, controller_name: &str, event: &str) -> Option<&'a HandlerDef> {
    controller(definition, controller_name)?.on.iter().find(|h| h.event.value == event)
}

pub fn rule<'a>(definition: &'a Definition, controller_name: &str, event: &str, ordinal: u32) -> Option<&'a RuleDef> {
    handler(definition, controller_name, event)?.rules.get(usize::try_from(ordinal).ok()?)
}

/// The state at dotted `path` within `machine`.
pub fn state<'a>(machine: &'a MachineDef, path: &str) -> Option<&'a StateDef> {
    let mut parts = path.split('.');
    let first = parts.next()?;
    let mut current = machine.states.iter().find(|s| s.name.value == first)?;
    for part in parts {
        current = current.states.iter().find(|s| s.name.value == part)?;
    }
    Some(current)
}

/// The parent path of a dotted path (`None` for a top-level state).
pub fn parent_path(path: &str) -> Option<&str> {
    path.rsplit_once('.').map(|(parent, _)| parent)
}

/// `parent.name`, or `name` at the top level.
pub fn join_path(parent: Option<&str>, name: &str) -> String {
    match parent {
        Some(parent) => format!("{parent}.{name}"),
        None => name.to_owned(),
    }
}

/// Every state name in the machine, at any depth. New states avoid all of
/// them so local-name references stay unambiguous.
pub fn state_names(machine: &MachineDef) -> BTreeSet<String> {
    fn walk(states: &[StateDef], out: &mut BTreeSet<String>) {
        for s in states {
            out.insert(s.name.value.clone());
            walk(&s.states, out);
        }
    }
    let mut out = BTreeSet::new();
    walk(&machine.states, &mut out);
    out
}

pub fn machine_names(definition: &Definition) -> BTreeSet<String> {
    definition.machines.iter().map(|m| m.name.value.clone()).collect()
}

pub fn controller_names(definition: &Definition) -> BTreeSet<String> {
    definition.controllers.iter().map(|c| c.name.value.clone()).collect()
}

pub fn external_names(definition: &Definition) -> BTreeSet<String> {
    definition.external.iter().map(|e| e.name.value.clone()).collect()
}

/// Every event name in use: declared, emitted or subscribed.
pub fn event_names(definition: &Definition) -> BTreeSet<String> {
    let declared = definition.events.iter().map(|e| e.name.value.clone());
    let emitted = definition
        .machines
        .iter()
        .flat_map(|m| m.transitions.iter())
        .flat_map(|t| t.emits.iter().map(|e| e.value.clone()));
    let subscribed = definition.controllers.iter().flat_map(|c| c.on.iter().map(|h| h.event.value.clone()));
    declared.chain(emitted).chain(subscribed).collect()
}

/// Every trigger name of `machine`: accepted by a transition, fired by a
/// rule, or exposed by a source.
pub fn trigger_names(definition: &Definition, machine_name: &str) -> BTreeSet<String> {
    let accepted =
        machine(definition, machine_name).into_iter().flat_map(|m| m.transitions.iter().map(|t| t.on.value.clone()));
    let fired = definition
        .controllers
        .iter()
        .flat_map(|c| c.on.iter())
        .flat_map(|h| h.rules.iter())
        .filter(|r| r.fire.value.machine == machine_name)
        .map(|r| r.fire.value.trigger.clone());
    let exposed = definition
        .external
        .iter()
        .flat_map(|e| e.triggers.iter())
        .filter(|t| t.value.machine == machine_name)
        .map(|t| t.value.trigger.clone());
    accepted.chain(fired).chain(exposed).collect()
}

/// Whether the file declares its events (`events:` non-empty), so every new
/// event must be declared too.
pub fn strict_events(definition: &Definition) -> bool {
    !definition.events.is_empty()
}

/// `capture_ok` → `CaptureOk`: the event-name style for a trigger.
pub fn pascal_case(name: &str) -> String {
    name.split(['_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect()
}

/// Split a comma-separated list, trimming blanks.
pub fn split_list(text: &str) -> Vec<String> {
    text.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

pub fn join_list<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    items.into_iter().collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "\
events:
  Paid: { payload: [orderId] }
machines:
  Order:
    states:
      - draft
      - placed:
          states: [waiting, paid]
    transitions:
      - { from: draft, to: placed, on: place }
      - { from: placed.waiting, to: placed.paid, on: pay, emits: [Paid] }
  Shipment:
    states: [idle, moving]
    transitions:
      - { from: idle, to: moving, on: start }
controllers:
  Fulfil:
    on:
      Paid:
        - fire: Shipment.start
external:
  Clock: [Order.tick]
";

    fn def() -> Definition {
        cascade_core::parse_definition(TEXT).expect("parses")
    }

    #[test]
    fn states_by_path() {
        let d = def();
        let order = machine(&d, "Order").expect("machine");
        assert_eq!(state(order, "placed.paid").map(|s| s.name.value.as_str()), Some("paid"));
        assert_eq!(state(order, "placed").map(|s| s.states.len()), Some(2));
        assert!(state(order, "paid").is_none(), "paths are full paths");
        assert!(state(order, "").is_none());
        assert_eq!(parent_path("placed.paid"), Some("placed"));
        assert_eq!(parent_path("draft"), None);
        assert_eq!(join_path(Some("a.b"), "c"), "a.b.c");
        assert_eq!(join_path(None, "c"), "c");
    }

    #[test]
    fn names_in_use() {
        let d = def();
        let order = machine(&d, "Order").expect("machine");
        assert_eq!(state_names(order).into_iter().collect::<Vec<_>>(), ["draft", "paid", "placed", "waiting"]);
        assert_eq!(machine_names(&d).into_iter().collect::<Vec<_>>(), ["Order", "Shipment"]);
        assert_eq!(controller_names(&d).into_iter().collect::<Vec<_>>(), ["Fulfil"]);
        assert_eq!(external_names(&d).into_iter().collect::<Vec<_>>(), ["Clock"]);
        assert_eq!(event_names(&d).into_iter().collect::<Vec<_>>(), ["Paid"]);
        assert_eq!(trigger_names(&d, "Order").into_iter().collect::<Vec<_>>(), ["pay", "place", "tick"]);
        assert_eq!(trigger_names(&d, "Shipment").into_iter().collect::<Vec<_>>(), ["start"]);
        assert!(strict_events(&d));
    }

    #[test]
    fn handlers_and_rules_by_key() {
        let d = def();
        assert_eq!(handler(&d, "Fulfil", "Paid").map(|h| h.rules.len()), Some(1));
        assert_eq!(rule(&d, "Fulfil", "Paid", 0).map(|r| r.fire.value.to_string()), Some("Shipment.start".into()));
        assert!(rule(&d, "Fulfil", "Paid", 1).is_none());
        assert!(handler(&d, "Nope", "Paid").is_none());
        assert_eq!(external(&d, "Clock").map(|e| e.triggers.len()), Some(1));
    }

    #[test]
    fn naming_helpers() {
        assert_eq!(pascal_case("capture_ok"), "CaptureOk");
        assert_eq!(pascal_case("go"), "Go");
        assert_eq!(pascal_case("retry-now"), "RetryNow");
        assert_eq!(split_list(" a, b ,,c "), ["a", "b", "c"]);
        assert!(split_list("  ").is_empty());
        assert_eq!(join_list(["a", "b"]), "a, b");
    }

    #[test]
    fn synthetic_elements_resolve_when_added() {
        let mut d = def();
        d.machines.push(machine_def("Extra", "idle"));
        d.controllers.push(controller_def("Router"));
        d.external.push(external_def("User"));
        assert!(cascade_core::resolve(d).is_ok());
    }
}
