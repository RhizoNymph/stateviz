//! Stable keys of definition elements, computed from the definition alone
//! (no resolved model), so they can be taken before and after each step of
//! a batch whose intermediate states need not resolve.
//!
//! Transition keys follow the resolver: one key per `from` source, paths
//! resolved as in [`StateIndex::resolve`], ordinals counting earlier
//! expansions with the same `(from, to, trigger)` in document order.

use super::refs::{StateIndex, join};
use crate::definition::{ControllerDef, Definition, HandlerDef, MachineDef, StateDef};
use crate::key::ElementKey;

/// One transition of the model: an expansion of a `transitions:` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Expansion {
    pub entry: usize,
    pub from: String,
    pub to: String,
    pub trigger: String,
    pub ordinal: u32,
}

impl Expansion {
    pub fn key(&self, machine: &str) -> ElementKey {
        ElementKey::Transition {
            machine: machine.to_owned(),
            from: self.from.clone(),
            to: self.to.clone(),
            trigger: self.trigger.clone(),
            ordinal: self.ordinal,
        }
    }
}

/// Every transition of a machine, in model order. References that do not
/// resolve produce no transition, as in the resolver.
pub(super) fn expansions(machine: &MachineDef) -> Vec<Expansion> {
    let index = StateIndex::of(&machine.states);
    let mut out: Vec<Expansion> = Vec::new();
    for (entry, t) in machine.transitions.iter().enumerate() {
        let Some(to) = index.resolve(&t.to.value) else { continue };
        for from in &t.from {
            let Some(from) = index.resolve(&from.value) else { continue };
            let earlier = out.iter().filter(|e| e.from == from && e.to == to && e.trigger == t.on.value).count();
            out.push(Expansion {
                entry,
                from: from.to_owned(),
                to: to.to_owned(),
                trigger: t.on.value.clone(),
                ordinal: u32::try_from(earlier).unwrap_or(u32::MAX),
            });
        }
    }
    out
}

/// Keys of the transitions expanded from entry `entry`.
pub(super) fn transition_keys(machine: &MachineDef, entry: usize) -> Vec<ElementKey> {
    entries_keys(machine, &[entry])
}

/// Keys of the transitions expanded from any of `entries`.
pub(super) fn entries_keys(machine: &MachineDef, entries: &[usize]) -> Vec<ElementKey> {
    expansions(machine).into_iter().filter(|e| entries.contains(&e.entry)).map(|e| e.key(&machine.name.value)).collect()
}

pub(super) fn machine_key(machine: &str) -> ElementKey {
    ElementKey::Machine { machine: machine.to_owned() }
}

pub(super) fn state_key(machine: &str, path: &str) -> ElementKey {
    ElementKey::State { machine: machine.to_owned(), path: path.to_owned() }
}

/// Keys of `state` (at `path`) and its descendants.
pub(super) fn subtree_keys(machine: &str, state: &StateDef, path: &str) -> Vec<ElementKey> {
    super::refs::subtree_paths(state, path).iter().map(|p| state_key(machine, p)).collect()
}

/// The machine, all its states and all its transitions.
pub(super) fn machine_keys(machine: &MachineDef) -> Vec<ElementKey> {
    let name = &machine.name.value;
    let mut out = vec![machine_key(name)];
    for s in &machine.states {
        out.extend(subtree_keys(name, s, &join(None, &s.name.value)));
    }
    out.extend(expansions(machine).iter().map(|e| e.key(name)));
    out
}

pub(super) fn rule_key(controller: &str, event: &str, index: usize) -> ElementKey {
    ElementKey::Rule {
        controller: controller.to_owned(),
        event: event.to_owned(),
        ordinal: u32::try_from(index).unwrap_or(u32::MAX),
    }
}

pub(super) fn handler_key(controller: &str, event: &str) -> ElementKey {
    ElementKey::Handler { controller: controller.to_owned(), event: event.to_owned() }
}

/// The handler and its rules.
pub(super) fn handler_keys(controller: &str, handler: &HandlerDef) -> Vec<ElementKey> {
    let event = &handler.event.value;
    std::iter::once(handler_key(controller, event))
        .chain((0..handler.rules.len()).map(|i| rule_key(controller, event, i)))
        .collect()
}

/// The controller, its handlers and their rules.
pub(super) fn controller_keys(controller: &ControllerDef) -> Vec<ElementKey> {
    let name = &controller.name.value;
    std::iter::once(ElementKey::Controller { controller: name.clone() })
        .chain(controller.on.iter().flat_map(|h| handler_keys(name, h)))
        .collect()
}

pub(super) fn event_key(event: &str) -> ElementKey {
    ElementKey::Event { event: event.to_owned() }
}

pub(super) fn external_key(source: &str) -> ElementKey {
    ElementKey::External { source: source.to_owned() }
}

pub(super) fn locate_transition(definition: &Definition, key: &ElementKey) -> Option<(String, usize)> {
    let ElementKey::Transition { machine, from, to, trigger, ordinal } = key else {
        return None;
    };
    let mdef = definition.machines.iter().find(|m| m.name.value == *machine)?;
    expansions(mdef)
        .into_iter()
        .find(|e| e.from == *from && e.to == *to && e.trigger == *trigger && e.ordinal == *ordinal)
        .map(|e| (machine.clone(), e.entry))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_definition;

    fn machine(text: &str) -> MachineDef {
        match parse_definition(text) {
            Ok(mut def) => def.machines.remove(0),
            Err(err) => panic!("{err}"),
        }
    }

    #[test]
    fn expansions_match_the_resolver() {
        let text = "machines:\n  M:\n    states:\n      - a\n      - p: { states: [b, c] }\n    transitions:\n      - { from: [a, b], to: c, on: go }\n      - { from: a, to: p.c, on: go }\n      - { from: nowhere, to: a, on: go }\n      - { from: p, to: a, on: back }\n";
        let m = machine(text);
        let found: Vec<(usize, String, u32)> = expansions(&m)
            .into_iter()
            .map(|e| (e.entry, format!("{}->{}@{}", e.from, e.to, e.trigger), e.ordinal))
            .collect();
        assert_eq!(
            found,
            [
                (0, "a->p.c@go".to_owned(), 0),
                (0, "p.b->p.c@go".to_owned(), 0),
                (1, "a->p.c@go".to_owned(), 1),
                (3, "p->a@back".to_owned(), 0),
            ]
        );
        let model = match crate::load_str(&text.replace("      - { from: nowhere, to: a, on: go }\n", "")) {
            Ok(model) => model,
            Err(err) => panic!("{err}"),
        };
        let from_model: Vec<String> =
            model.transition_ids().map(|t| model.key_of(crate::key::ElementRef::Transition(t)).to_string()).collect();
        let edited = machine(&text.replace("      - { from: nowhere, to: a, on: go }\n", ""));
        let ours: Vec<String> = expansions(&edited).iter().map(|e| e.key("M").to_string()).collect();
        assert_eq!(ours, from_model);
    }

    #[test]
    fn owned_keys() {
        let m = machine("machines:\n  M:\n    states: [a, b]\n    transitions: [{ from: a, to: b, on: go }]\n");
        let keys: Vec<String> = machine_keys(&m).iter().map(ToString::to_string).collect();
        assert_eq!(keys, ["machine:M", "state:M:a", "state:M:b", "transition:M:a->b@go"]);
    }
}
