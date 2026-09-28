//! The resolved model: typed arenas plus name indexes.
//!
//! A [`Model`] is only built by [`crate::resolve`], which guarantees that
//! every id stored anywhere in it is in range and that the reverse indexes
//! agree with the forward references. Accessors therefore index directly.

mod elements;

use std::collections::HashMap;

pub use elements::{
    Controller, Event, ExternalSource, Handler, Machine, Rule, State, StateKind, Target, Transition, Trigger,
};

use crate::definition::Definition;
use crate::ids::{ControllerId, EventId, ExternalId, HandlerId, MachineId, RuleId, StateId, TransitionId, TriggerId};
use crate::key::{ElementKey, ElementRef};
use crate::span::SourceSpan;

#[derive(Clone, Debug)]
pub struct Model {
    pub(crate) definition: Definition,
    pub(crate) machines: Vec<Machine>,
    pub(crate) states: Vec<State>,
    pub(crate) transitions: Vec<Transition>,
    pub(crate) triggers: Vec<Trigger>,
    pub(crate) events: Vec<Event>,
    pub(crate) controllers: Vec<Controller>,
    pub(crate) handlers: Vec<Handler>,
    pub(crate) rules: Vec<Rule>,
    pub(crate) externals: Vec<ExternalSource>,
    pub(crate) machine_names: HashMap<String, MachineId>,
    pub(crate) event_names: HashMap<String, EventId>,
    pub(crate) controller_names: HashMap<String, ControllerId>,
    pub(crate) external_names: HashMap<String, ExternalId>,
}

macro_rules! arena_accessors {
    ($one:ident, $all:ident, $ids:ident, $count:ident, $field:ident, $id:ty, $ty:ty) => {
        pub fn $one(&self, id: $id) -> &$ty {
            &self.$field[id.index()]
        }

        pub fn $all(&self) -> impl ExactSizeIterator<Item = ($id, &$ty)> + '_ {
            self.$field.iter().enumerate().map(|(i, x)| (<$id>::new(i), x))
        }

        pub fn $ids(&self) -> impl ExactSizeIterator<Item = $id> + use<> {
            (0..self.$field.len()).map(<$id>::new)
        }

        pub fn $count(&self) -> usize {
            self.$field.len()
        }
    };
}

impl Model {
    arena_accessors!(machine, machines, machine_ids, machine_count, machines, MachineId, Machine);
    arena_accessors!(state, states, state_ids, state_count, states, StateId, State);
    arena_accessors!(transition, transitions, transition_ids, transition_count, transitions, TransitionId, Transition);
    arena_accessors!(trigger, triggers, trigger_ids, trigger_count, triggers, TriggerId, Trigger);
    arena_accessors!(event, events, event_ids, event_count, events, EventId, Event);
    arena_accessors!(controller, controllers, controller_ids, controller_count, controllers, ControllerId, Controller);
    arena_accessors!(handler, handlers, handler_ids, handler_count, handlers, HandlerId, Handler);
    arena_accessors!(rule, rules, rule_ids, rule_count, rules, RuleId, Rule);
    arena_accessors!(external, externals, external_ids, external_count, externals, ExternalId, ExternalSource);

    /// The definition this model was resolved from.
    pub fn definition(&self) -> &Definition {
        &self.definition
    }

    // --- Name lookup ------------------------------------------------------

    pub fn machine_by_name(&self, name: &str) -> Option<MachineId> {
        self.machine_names.get(name).copied()
    }

    pub fn event_by_name(&self, name: &str) -> Option<EventId> {
        self.event_names.get(name).copied()
    }

    pub fn controller_by_name(&self, name: &str) -> Option<ControllerId> {
        self.controller_names.get(name).copied()
    }

    pub fn external_by_name(&self, name: &str) -> Option<ExternalId> {
        self.external_names.get(name).copied()
    }

    pub fn trigger_by_name(&self, machine: MachineId, name: &str) -> Option<TriggerId> {
        self.machine(machine).triggers.iter().copied().find(|&t| self.trigger(t).name == name)
    }

    /// Look a state up by its full dotted path.
    pub fn state_by_path(&self, machine: MachineId, path: &str) -> Option<StateId> {
        self.machine(machine).states.iter().copied().find(|&s| self.state(s).path == path)
    }

    pub fn handler_for(&self, controller: ControllerId, event: EventId) -> Option<HandlerId> {
        self.controller(controller).handlers.iter().copied().find(|&h| self.handler(h).event == event)
    }

    // --- Statechart structure ---------------------------------------------

    /// Ancestors of `state`, nearest first, excluding `state` itself.
    pub fn ancestors(&self, state: StateId) -> impl Iterator<Item = StateId> + '_ {
        std::iter::successors(self.state(state).parent, |&s| self.state(s).parent)
    }

    /// Whether `ancestor` is `state` or one of its ancestors.
    pub fn is_ancestor_or_self(&self, ancestor: StateId, state: StateId) -> bool {
        ancestor == state || self.ancestors(state).any(|a| a == ancestor)
    }

    /// The atomic state entered when `state` is entered by default: follow
    /// initial children down from a compound state. Atomic, final and
    /// history states return themselves.
    pub fn default_entry(&self, state: StateId) -> StateId {
        let mut current = state;
        while let StateKind::Compound { initial, .. } = &self.state(current).kind {
            current = *initial;
        }
        current
    }

    /// The transitions an instance in `state` takes for `trigger`, with
    /// statechart priority: transitions declared on the innermost state win,
    /// so the result holds the candidates from the deepest state (the state
    /// itself, then its ancestors) that has any. More than one candidate
    /// means the choice depends on guards.
    pub fn enabled_transitions(&self, state: StateId, trigger: TriggerId) -> Vec<TransitionId> {
        let accepted = &self.trigger(trigger).accepted_by;
        std::iter::once(state)
            .chain(self.ancestors(state))
            .map(|s| accepted.iter().copied().filter(|&t| self.transition(t).from == s).collect::<Vec<_>>())
            .find(|candidates| !candidates.is_empty())
            .unwrap_or_default()
    }

    // --- Presentation helpers ---------------------------------------------

    /// `Order: pending → paid`, the label transitions are known by.
    pub fn transition_label(&self, id: TransitionId) -> String {
        let t = self.transition(id);
        format!("{}: {} → {}", self.machine(t.machine).name, self.state(t.from).path, self.state(t.to).path)
    }

    /// A short human-readable label for any element.
    pub fn label_of(&self, element: ElementRef) -> String {
        match element {
            ElementRef::Machine(id) => self.machine(id).name.clone(),
            ElementRef::State(id) => {
                let s = self.state(id);
                format!("{}.{}", self.machine(s.machine).name, s.path)
            }
            ElementRef::Transition(id) => self.transition_label(id),
            ElementRef::Trigger(id) => {
                let t = self.trigger(id);
                format!("{}.{}", self.machine(t.machine).name, t.name)
            }
            ElementRef::Event(id) => self.event(id).name.clone(),
            ElementRef::Controller(id) => self.controller(id).name.clone(),
            ElementRef::Handler(id) => {
                let h = self.handler(id);
                format!("{} on {}", self.controller(h.controller).name, self.event(h.event).name)
            }
            ElementRef::Rule(id) => {
                let r = self.rule(id);
                let t = self.trigger(r.trigger);
                format!(
                    "{} on {}: fire {}.{}",
                    self.controller(r.controller).name,
                    self.event(r.event).name,
                    self.machine(t.machine).name,
                    t.name
                )
            }
            ElementRef::External(id) => self.external(id).name.clone(),
        }
    }

    /// Where the element is defined, for click-to-source.
    pub fn span_of(&self, element: ElementRef) -> SourceSpan {
        match element {
            ElementRef::Machine(id) => self.machine(id).span,
            ElementRef::State(id) => self.state(id).span,
            ElementRef::Transition(id) => self.transition(id).span,
            ElementRef::Trigger(id) => self.trigger(id).span,
            ElementRef::Event(id) => self.event(id).span,
            ElementRef::Controller(id) => self.controller(id).span,
            ElementRef::Handler(id) => self.handler(id).span,
            ElementRef::Rule(id) => self.rule(id).span,
            ElementRef::External(id) => self.external(id).span,
        }
    }

    /// The machine an element belongs to. Rules and handlers belong to the
    /// machine they fire into only indirectly, so they return `None`, as do
    /// events, controllers and external sources.
    pub fn machine_of(&self, element: ElementRef) -> Option<MachineId> {
        match element {
            ElementRef::Machine(id) => Some(id),
            ElementRef::State(id) => Some(self.state(id).machine),
            ElementRef::Transition(id) => Some(self.transition(id).machine),
            ElementRef::Trigger(id) => Some(self.trigger(id).machine),
            ElementRef::Event(_)
            | ElementRef::Controller(_)
            | ElementRef::Handler(_)
            | ElementRef::Rule(_)
            | ElementRef::External(_) => None,
        }
    }

    // --- Stable keys ------------------------------------------------------

    pub fn key_of(&self, element: ElementRef) -> ElementKey {
        match element {
            ElementRef::Machine(id) => ElementKey::Machine { machine: self.machine(id).name.clone() },
            ElementRef::State(id) => {
                let s = self.state(id);
                ElementKey::State { machine: self.machine(s.machine).name.clone(), path: s.path.clone() }
            }
            ElementRef::Transition(id) => {
                let t = self.transition(id);
                ElementKey::Transition {
                    machine: self.machine(t.machine).name.clone(),
                    from: self.state(t.from).path.clone(),
                    to: self.state(t.to).path.clone(),
                    trigger: self.trigger(t.trigger).name.clone(),
                    ordinal: t.ordinal,
                }
            }
            ElementRef::Trigger(id) => {
                let t = self.trigger(id);
                ElementKey::Trigger { machine: self.machine(t.machine).name.clone(), trigger: t.name.clone() }
            }
            ElementRef::Event(id) => ElementKey::Event { event: self.event(id).name.clone() },
            ElementRef::Controller(id) => ElementKey::Controller { controller: self.controller(id).name.clone() },
            ElementRef::Handler(id) => {
                let h = self.handler(id);
                ElementKey::Handler {
                    controller: self.controller(h.controller).name.clone(),
                    event: self.event(h.event).name.clone(),
                }
            }
            ElementRef::Rule(id) => {
                let r = self.rule(id);
                ElementKey::Rule {
                    controller: self.controller(r.controller).name.clone(),
                    event: self.event(r.event).name.clone(),
                    ordinal: r.ordinal,
                }
            }
            ElementRef::External(id) => ElementKey::External { source: self.external(id).name.clone() },
        }
    }

    /// Find the element a stable key names in this model, if it exists.
    pub fn resolve_key(&self, key: &ElementKey) -> Option<ElementRef> {
        match key {
            ElementKey::Machine { machine } => self.machine_by_name(machine).map(ElementRef::Machine),
            ElementKey::State { machine, path } => {
                let m = self.machine_by_name(machine)?;
                self.state_by_path(m, path).map(ElementRef::State)
            }
            ElementKey::Transition { machine, from, to, trigger, ordinal } => {
                let m = self.machine_by_name(machine)?;
                let from = self.state_by_path(m, from)?;
                let to = self.state_by_path(m, to)?;
                let trigger = self.trigger_by_name(m, trigger)?;
                self.machine(m)
                    .transitions
                    .iter()
                    .copied()
                    .find(|&t| {
                        let t = self.transition(t);
                        t.from == from && t.to == to && t.trigger == trigger && t.ordinal == *ordinal
                    })
                    .map(ElementRef::Transition)
            }
            ElementKey::Trigger { machine, trigger } => {
                let m = self.machine_by_name(machine)?;
                self.trigger_by_name(m, trigger).map(ElementRef::Trigger)
            }
            ElementKey::Event { event } => self.event_by_name(event).map(ElementRef::Event),
            ElementKey::Controller { controller } => self.controller_by_name(controller).map(ElementRef::Controller),
            ElementKey::Handler { controller, event } => {
                let c = self.controller_by_name(controller)?;
                let e = self.event_by_name(event)?;
                self.handler_for(c, e).map(ElementRef::Handler)
            }
            ElementKey::Rule { controller, event, ordinal } => {
                let c = self.controller_by_name(controller)?;
                let e = self.event_by_name(event)?;
                let h = self.handler_for(c, e)?;
                self.handler(h).rules.iter().copied().find(|&r| self.rule(r).ordinal == *ordinal).map(ElementRef::Rule)
            }
            ElementKey::External { source } => self.external_by_name(source).map(ElementRef::External),
        }
    }

    /// Every element in the model, in a deterministic order: machines, then
    /// per machine its states, triggers and transitions, then events,
    /// controllers with their handlers and rules, then external sources.
    pub fn all_elements(&self) -> Vec<ElementRef> {
        let mut out = Vec::new();
        for (mid, machine) in self.machines() {
            out.push(ElementRef::Machine(mid));
            out.extend(machine.states.iter().copied().map(ElementRef::State));
            out.extend(machine.triggers.iter().copied().map(ElementRef::Trigger));
            out.extend(machine.transitions.iter().copied().map(ElementRef::Transition));
        }
        out.extend(self.event_ids().map(ElementRef::Event));
        for (cid, controller) in self.controllers() {
            out.push(ElementRef::Controller(cid));
            for &h in &controller.handlers {
                out.push(ElementRef::Handler(h));
                out.extend(self.handler(h).rules.iter().copied().map(ElementRef::Rule));
            }
        }
        out.extend(self.external_ids().map(ElementRef::External));
        out
    }
}
