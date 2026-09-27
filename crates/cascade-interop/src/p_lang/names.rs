//! P identifiers for every model element.
//!
//! P identifiers are `[A-Za-z_][A-Za-z0-9_]*` and must not be keywords.
//! Machines, controllers, events, types and tests share one global
//! namespace; states are per machine. Names are sanitized (other characters
//! become `_`), keywords get a trailing `_`, and collisions a `_2`, `_3`, …
//! suffix. The generated names (`tRegistry`, `eWire`, `TestDriver`,
//! `tcSystem`, the `Wiring`/`Running` states and the `registry` variable)
//! are reserved first, so a Cascade machine called `TestDriver` becomes
//! `TestDriver_2`.

use std::collections::HashSet;

use cascade_core::model::StateKind;
use cascade_core::{ControllerId, EventId, MachineId, Model, StateId, TriggerId};

pub(super) const REGISTRY_TYPE: &str = "tRegistry";
pub(super) const WIRE_EVENT: &str = "eWire";
pub(super) const DRIVER: &str = "TestDriver";
pub(super) const TEST: &str = "tcSystem";
pub(super) const WIRING_STATE: &str = "Wiring";
pub(super) const RUNNING_STATE: &str = "Running";

/// P keywords and built-in type names (P is case-sensitive).
const KEYWORDS: &[&str] = &[
    "announce",
    "any",
    "as",
    "assert",
    "assume",
    "bool",
    "break",
    "case",
    "choose",
    "cold",
    "compose",
    "continue",
    "data",
    "default",
    "defer",
    "do",
    "else",
    "entry",
    "enum",
    "event",
    "exit",
    "false",
    "float",
    "foreach",
    "foreign",
    "format",
    "fun",
    "goto",
    "halt",
    "hidee",
    "hidei",
    "hot",
    "if",
    "ignore",
    "implementation",
    "in",
    "int",
    "keys",
    "machine",
    "main",
    "map",
    "module",
    "new",
    "null",
    "observes",
    "on",
    "param",
    "print",
    "private",
    "raise",
    "receive",
    "refines",
    "rename",
    "return",
    "safe",
    "send",
    "seq",
    "set",
    "sizeof",
    "spec",
    "start",
    "state",
    "string",
    "test",
    "this",
    "to",
    "true",
    "type",
    "union",
    "values",
    "var",
    "while",
    "with",
];

/// Replace characters outside `[A-Za-z0-9_]` with `_`; never empty, never
/// starting with a digit.
pub(super) fn sanitize(text: &str) -> String {
    let mut out: String = text.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// One namespace of identifiers.
#[derive(Debug, Default)]
pub(super) struct Idents {
    used: HashSet<String>,
}

impl Idents {
    pub(super) fn reserve(&mut self, name: &str) {
        self.used.insert(name.to_owned());
    }

    /// A fresh identifier based on `base`.
    pub(super) fn allocate(&mut self, base: &str) -> String {
        let mut base = sanitize(base);
        if KEYWORDS.contains(&base.as_str()) {
            base.push('_');
        }
        if self.used.insert(base.clone()) {
            return base;
        }
        let mut n = 2u32;
        loop {
            let candidate = format!("{base}_{n}");
            if self.used.insert(candidate.clone()) {
                return candidate;
            }
            n = n.saturating_add(1);
        }
    }
}

/// The P name of every element, indexed by the element's id.
#[derive(Debug)]
pub(super) struct Names {
    machines: Vec<String>,
    controllers: Vec<String>,
    events: Vec<String>,
    /// The named-tuple payload type, for events with a declared payload.
    payload_types: Vec<Option<String>>,
    payload_fields: Vec<Vec<String>>,
    /// The P event that delivers each trigger: `e<Machine>_<trigger>`.
    triggers: Vec<String>,
    /// Only atomic and final states become P states.
    states: Vec<Option<String>>,
}

impl Names {
    pub(super) fn allocate(model: &Model) -> Self {
        let mut global = Idents::default();
        for fixed in [REGISTRY_TYPE, WIRE_EVENT, DRIVER, TEST] {
            global.reserve(fixed);
        }
        let machines: Vec<String> = model.machines().map(|(_, m)| global.allocate(&m.name)).collect();
        let controllers = model.controllers().map(|(_, c)| global.allocate(&c.name)).collect();

        let mut events = Vec::with_capacity(model.event_count());
        let mut payload_types = Vec::with_capacity(model.event_count());
        let mut payload_fields = Vec::with_capacity(model.event_count());
        for (_, event) in model.events() {
            let name = global.allocate(&event.name);
            if event.payload.is_empty() {
                payload_types.push(None);
                payload_fields.push(Vec::new());
            } else {
                payload_types.push(Some(global.allocate(&format!("t{name}"))));
                let mut fields = Idents::default();
                payload_fields.push(event.payload.iter().map(|f| fields.allocate(f)).collect());
            }
            events.push(name);
        }

        let triggers = model
            .triggers()
            .map(|(_, t)| {
                let machine = machines.get(t.machine.index()).map_or("", String::as_str);
                global.allocate(&format!("e{machine}_{}", sanitize(&t.name)))
            })
            .collect();

        let mut states = vec![None; model.state_count()];
        for (_, machine) in model.machines() {
            let mut local = Idents::default();
            // States share the machine scope with its `registry` variable.
            local.reserve(WIRING_STATE);
            local.reserve("registry");
            for &s in &machine.states {
                let state = model.state(s);
                if matches!(state.kind, StateKind::Atomic | StateKind::Final) {
                    states[s.index()] = Some(local.allocate(&state.path.replace('.', "_")));
                }
            }
        }

        Self { machines, controllers, events, payload_types, payload_fields, triggers, states }
    }

    pub(super) fn machine(&self, id: MachineId) -> &str {
        self.machines.get(id.index()).map_or("", String::as_str)
    }

    pub(super) fn controller(&self, id: ControllerId) -> &str {
        self.controllers.get(id.index()).map_or("", String::as_str)
    }

    pub(super) fn event(&self, id: EventId) -> &str {
        self.events.get(id.index()).map_or("", String::as_str)
    }

    pub(super) fn payload_type(&self, id: EventId) -> Option<&str> {
        self.payload_types.get(id.index()).and_then(|t| t.as_deref())
    }

    pub(super) fn payload_fields(&self, id: EventId) -> &[String] {
        self.payload_fields.get(id.index()).map_or(&[], Vec::as_slice)
    }

    pub(super) fn trigger(&self, id: TriggerId) -> &str {
        self.triggers.get(id.index()).map_or("", String::as_str)
    }

    /// The P state for an atomic or final state; `None` for compound and
    /// history states, which are never current.
    pub(super) fn state(&self, id: StateId) -> Option<&str> {
        self.states.get(id.index()).and_then(|s| s.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_sanitized_keyword_safe_and_unique() {
        let mut ids = Idents::default();
        ids.reserve("TestDriver");
        assert_eq!(ids.allocate("draft-2"), "draft_2");
        assert_eq!(ids.allocate("state"), "state_");
        assert_eq!(ids.allocate("TestDriver"), "TestDriver_2");
        assert_eq!(ids.allocate("draft.2"), "draft_2_2");
        assert_eq!(ids.allocate("9lives"), "_9lives");
        assert_eq!(ids.allocate("état"), "_tat");
    }
}
