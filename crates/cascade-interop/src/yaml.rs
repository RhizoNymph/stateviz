//! Writing a definition back to YAML in the spec's compact style.
//!
//! - Transitions are one flow mapping per line, with `from`, `to` and `on`
//!   aligned per machine: `- { from: a, to: b, on: go, emits: [E] }`.
//!   Multi-source `from` lists are written as they were (`from: [a, b]`).
//! - States use the flow list `states: [a, b]` when no sibling has a body;
//!   otherwise a block list where plain states are `- name`, states with
//!   only a kind are `- name: { kind: final }`, and compound states are a
//!   block mapping with `initial:` and nested `states:`.
//! - Rules are block mappings; events, external sources and fields are flow
//!   collections.
//! - Strings are quoted only when a YAML parser would read them differently
//!   (see [`quote`]).
//!
//! `parse_definition(to_yaml(def))` yields a definition that resolves to
//! the same model as `def` (spans aside); the tests check this for every
//! example and every import fixture.

mod quote;

use std::fmt::Write as _;

use cascade_core::definition::{
    ControllerDef, EventDef, ExternalDef, HandlerDef, MachineDef, RuleDef, StateDef, StateKindDef, TargetMode,
    TargetSpec, TransitionDef, ValueExpr,
};
use cascade_core::{Definition, Spanned};

use quote::{Ctx, scalar};

/// The definition as YAML text in the spec's compact style.
pub fn to_yaml(definition: &Definition) -> String {
    let mut out = String::new();
    if let Some(system) = &definition.system {
        line(&mut out, 0, &format!("system: {}", scalar(&system.value, Ctx::Block)));
        out.push('\n');
    }
    write_machines(&mut out, &definition.machines);
    if !definition.events.is_empty() {
        out.push('\n');
        write_events(&mut out, &definition.events);
    }
    if !definition.controllers.is_empty() {
        out.push('\n');
        write_controllers(&mut out, &definition.controllers);
    }
    if !definition.external.is_empty() {
        out.push('\n');
        write_external(&mut out, &definition.external);
    }
    out
}

fn line(out: &mut String, indent: usize, text: &str) {
    for _ in 0..indent {
        out.push(' ');
    }
    out.push_str(text);
    out.push('\n');
}

fn flow_list<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    let items: Vec<String> = items.into_iter().map(|s| scalar(s, Ctx::Flow).into_owned()).collect();
    format!("[{}]", items.join(", "))
}

fn values(items: &[Spanned<String>]) -> impl Iterator<Item = &str> {
    items.iter().map(|s| s.value.as_str())
}

// --- Machines -----------------------------------------------------------------

fn write_machines(out: &mut String, machines: &[MachineDef]) {
    if machines.is_empty() {
        line(out, 0, "machines: {}");
        return;
    }
    line(out, 0, "machines:");
    for (i, machine) in machines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        write_machine(out, machine);
    }
}

fn write_machine(out: &mut String, machine: &MachineDef) {
    line(out, 2, &format!("{}:", scalar(&machine.name.value, Ctx::Block)));
    if let Some(color) = &machine.color {
        line(out, 4, &format!("color: {}", color.value.name()));
    }
    if let Some(domain) = &machine.domain {
        line(out, 4, &format!("domain: {}", scalar(&domain.value, Ctx::Block)));
    }
    if let Some(initial) = &machine.initial {
        line(out, 4, &format!("initial: {}", scalar(&initial.value, Ctx::Block)));
    }
    if !machine.fields.is_empty() {
        line(out, 4, &format!("fields: {}", flow_list(values(&machine.fields))));
    }
    write_states(out, 4, &machine.states);
    if !machine.transitions.is_empty() {
        line(out, 4, "transitions:");
        for row in align_transitions(&machine.transitions) {
            line(out, 6, &row);
        }
    }
}

fn has_body(state: &StateDef) -> bool {
    state.kind.value != StateKindDef::Normal || state.initial.is_some() || !state.states.is_empty()
}

/// `states:` at `indent`, in flow form when no state has a body.
fn write_states(out: &mut String, indent: usize, states: &[StateDef]) {
    if !states.iter().any(has_body) {
        line(out, indent, &format!("states: {}", flow_list(states.iter().map(|s| s.name.value.as_str()))));
        return;
    }
    line(out, indent, "states:");
    for state in states {
        write_state_item(out, indent + 2, state);
    }
}

fn write_state_item(out: &mut String, indent: usize, state: &StateDef) {
    let name = scalar(&state.name.value, Ctx::Block);
    let kind = (state.kind.value != StateKindDef::Normal).then(|| format!("kind: {}", state.kind.value.name()));
    if !has_body(state) {
        line(out, indent, &format!("- {name}"));
    } else if state.initial.is_none() && state.states.is_empty() {
        // Only a kind: keep it on one line.
        line(out, indent, &format!("- {name}: {{ {} }}", kind.unwrap_or_default()));
    } else {
        line(out, indent, &format!("- {name}:"));
        let body = indent + 4;
        if let Some(kind) = kind {
            line(out, body, &kind);
        }
        if let Some(initial) = &state.initial {
            line(out, body, &format!("initial: {}", scalar(&initial.value, Ctx::Block)));
        }
        if !state.states.is_empty() {
            write_states(out, body, &state.states);
        }
    }
}

/// Transition rows with `from`, `to` and `on` padded into columns.
fn align_transitions(transitions: &[TransitionDef]) -> Vec<String> {
    struct Row {
        from: String,
        to: String,
        on: String,
        rest: Vec<String>,
    }
    let rows: Vec<Row> = transitions
        .iter()
        .map(|t| {
            let from = match t.from.as_slice() {
                [one] => scalar(&one.value, Ctx::Flow).into_owned(),
                many => flow_list(values(many)),
            };
            let mut rest = Vec::new();
            if let Some(guard) = &t.guard {
                rest.push(format!("guard: {}", scalar(&guard.value, Ctx::Flow)));
            }
            if !t.emits.is_empty() {
                rest.push(format!("emits: {}", flow_list(values(&t.emits))));
            }
            if t.bounded {
                rest.push("bounded: true".to_owned());
            }
            Row {
                from: format!("from: {from},"),
                to: format!("to: {},", scalar(&t.to.value, Ctx::Flow)),
                on: format!("on: {}", scalar(&t.on.value, Ctx::Flow)),
                rest,
            }
        })
        .collect();
    let width = |f: fn(&Row) -> usize, only_with_rest: bool| {
        rows.iter().filter(|r| !only_with_rest || !r.rest.is_empty()).map(f).max().unwrap_or(0)
    };
    let from_w = width(|r| r.from.chars().count(), false);
    let to_w = width(|r| r.to.chars().count(), false);
    let on_w = width(|r| r.on.chars().count() + 1, true);
    rows.iter()
        .map(|r| {
            let mut text = format!("- {{ {:from_w$} {:to_w$} ", r.from, r.to);
            if r.rest.is_empty() {
                let _ = write!(text, "{} }}", r.on);
            } else {
                let _ = write!(text, "{:on_w$} {} }}", format!("{},", r.on), r.rest.join(", "));
            }
            text
        })
        .collect()
}

// --- Events -------------------------------------------------------------------

fn write_events(out: &mut String, events: &[EventDef]) {
    line(out, 0, "events:");
    for event in events {
        let body = if event.payload.is_empty() {
            "{}".to_owned()
        } else {
            format!("{{ payload: {} }}", flow_list(values(&event.payload)))
        };
        line(out, 2, &format!("{}: {body}", scalar(&event.name.value, Ctx::Block)));
    }
}

// --- Controllers ----------------------------------------------------------------

fn write_controllers(out: &mut String, controllers: &[ControllerDef]) {
    line(out, 0, "controllers:");
    for (i, controller) in controllers.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        line(out, 2, &format!("{}:", scalar(&controller.name.value, Ctx::Block)));
        if controller.on.is_empty() {
            line(out, 4, "on: {}");
            continue;
        }
        line(out, 4, "on:");
        for handler in &controller.on {
            write_handler(out, handler);
        }
    }
}

fn write_handler(out: &mut String, handler: &HandlerDef) {
    let event = scalar(&handler.event.value, Ctx::Block);
    if handler.rules.is_empty() {
        line(out, 6, &format!("{event}: []"));
        return;
    }
    line(out, 6, &format!("{event}:"));
    for rule in &handler.rules {
        write_rule(out, rule);
    }
}

fn write_rule(out: &mut String, rule: &RuleDef) {
    line(out, 8, &format!("- fire: {}", scalar(&rule.fire.value.to_string(), Ctx::Block)));
    if let Some(target) = &rule.target {
        line(out, 10, &format!("target: {}", scalar(&selector(&target.value), Ctx::Block)));
    }
    if let Some(when) = &rule.when {
        line(out, 10, &format!("when: {}", scalar(&when.value, Ctx::Block)));
    }
    if rule.bounded {
        line(out, 10, "bounded: true");
    }
}

/// A target selector in the syntax `parse_target` reads. Unlike the core
/// `Display`, literals are quoted whenever the selector tokenizer would read
/// them differently (e.g. a literal that starts with `event.`).
fn selector(spec: &TargetSpec) -> String {
    let mut text = String::new();
    match spec.mode {
        TargetMode::One => {}
        TargetMode::All => text.push_str("all "),
        TargetMode::Spawn => text.push_str("new "),
    }
    text.push_str(&spec.machine);
    let (lead, joiner, op) = match spec.mode {
        TargetMode::Spawn => (" with ", ", ", "="),
        TargetMode::One | TargetMode::All => (" where ", " and ", "=="),
    };
    for (i, clause) in spec.clauses.iter().enumerate() {
        text.push_str(if i == 0 { lead } else { joiner });
        let _ = write!(text, "{} {op} {}", clause.field, selector_value(&clause.value));
    }
    text
}

fn selector_value(value: &ValueExpr) -> String {
    match value {
        ValueExpr::EventField(field) => format!("event.{field}"),
        ValueExpr::Literal(lit) => {
            let word = !lit.is_empty()
                && lit.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
                && !lit.starts_with("event.");
            if word {
                lit.clone()
            } else if lit.contains('"') {
                format!("'{lit}'")
            } else {
                format!("\"{lit}\"")
            }
        }
    }
}

// --- External sources -------------------------------------------------------------

fn write_external(out: &mut String, sources: &[ExternalDef]) {
    line(out, 0, "external:");
    for source in sources {
        let triggers: Vec<String> = source.triggers.iter().map(|t| t.value.to_string()).collect();
        line(
            out,
            2,
            &format!("{}: {}", scalar(&source.name.value, Ctx::Block), flow_list(triggers.iter().map(String::as_str))),
        );
    }
}
