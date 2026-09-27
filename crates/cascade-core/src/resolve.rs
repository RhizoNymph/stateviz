//! [`Definition`] → [`Model`]: resolve every name to a typed id, build the
//! statechart structure and the reverse indexes, and report every dangling
//! or contradictory reference.

use std::collections::{HashMap, HashSet};

use crate::definition::{Definition, FieldClause, MachineDef, StateDef, StateKindDef, TargetMode, ValueExpr};
use crate::error::{Diagnostic, DiagnosticKind, LoadError};
use crate::ids::{ControllerId, EventId, ExternalId, HandlerId, MachineId, RuleId, StateId, TransitionId, TriggerId};
use crate::model::{
    Controller, Event, ExternalSource, Handler, Machine, Model, Rule, State, StateKind, Target, Transition, Trigger,
};
use crate::span::{SourceSpan, Spanned};

/// Resolve a parsed or imported definition into a model.
pub fn resolve(definition: Definition) -> Result<Model, LoadError> {
    let mut r = Resolver { strict_events: !definition.events.is_empty(), ..Resolver::default() };

    for mdef in &definition.machines {
        r.add_machine(mdef);
    }
    for edef in &definition.events {
        if r.model.event_names.contains_key(&edef.name.value) {
            r.duplicate("events", "event", &edef.name);
            continue;
        }
        let id = EventId::new(r.model.events.len());
        r.model.event_names.insert(edef.name.value.clone(), id);
        r.model.events.push(Event {
            name: edef.name.value.clone(),
            payload: edef.payload.iter().map(|p| p.value.clone()).collect(),
            declared: true,
            emitted_by: Vec::new(),
            handlers: Vec::new(),
            span: edef.name.span,
        });
    }
    for mdef in &definition.machines {
        if let Some(mid) = r.model.machine_by_name(&mdef.name.value) {
            r.add_transitions(mid, mdef);
        }
    }
    r.add_controllers(&definition);
    r.add_externals(&definition);

    if r.diags.is_empty() {
        r.model.definition = definition;
        Ok(r.model)
    } else {
        r.diags.sort_by_key(|d| d.span);
        Err(LoadError { diagnostics: r.diags })
    }
}

struct Resolver {
    model: Model,
    diags: Vec<Diagnostic>,
    /// Machines that failed to resolve; references to them are not reported
    /// again as unknown.
    failed_machines: HashSet<String>,
    strict_events: bool,
}

impl Default for Resolver {
    fn default() -> Self {
        Self {
            model: Model {
                definition: Definition::default(),
                machines: Vec::new(),
                states: Vec::new(),
                transitions: Vec::new(),
                triggers: Vec::new(),
                events: Vec::new(),
                controllers: Vec::new(),
                handlers: Vec::new(),
                rules: Vec::new(),
                externals: Vec::new(),
                machine_names: HashMap::new(),
                event_names: HashMap::new(),
                controller_names: HashMap::new(),
                external_names: HashMap::new(),
            },
            diags: Vec::new(),
            failed_machines: HashSet::new(),
            strict_events: false,
        }
    }
}

impl Resolver {
    fn push(&mut self, kind: DiagnosticKind, span: SourceSpan) {
        self.diags.push(Diagnostic { kind, span });
    }

    fn duplicate(&mut self, context: &str, what: &'static str, name: &Spanned<String>) {
        self.push(DiagnosticKind::Duplicate { context: context.to_owned(), what, name: name.value.clone() }, name.span);
    }

    // --- Machines and states -----------------------------------------------

    fn add_machine(&mut self, mdef: &MachineDef) {
        let name = &mdef.name;
        if self.model.machine_names.contains_key(&name.value) || self.failed_machines.contains(&name.value) {
            self.duplicate("machines", "machine", name);
            return;
        }
        if mdef.states.is_empty() {
            self.push(DiagnosticKind::EmptyMachine { machine: name.value.clone() }, name.span);
            self.failed_machines.insert(name.value.clone());
            return;
        }

        let mid = MachineId::new(self.model.machines.len());
        self.model.machine_names.insert(name.value.clone(), mid);
        // Push a placeholder so states can refer to the machine id; the real
        // initial state is set once the states exist.
        self.model.machines.push(Machine {
            name: name.value.clone(),
            color: mdef.color.as_ref().map(|c| c.value),
            domain: mdef.domain.as_ref().map(|d| d.value.clone()),
            initial: StateId::new(self.model.states.len()),
            fields: mdef.fields.iter().map(|f| f.value.clone()).collect(),
            states: Vec::new(),
            top_states: Vec::new(),
            transitions: Vec::new(),
            triggers: Vec::new(),
            span: name.span,
        });

        let context = format!("machine `{}`", name.value);
        let top = self.add_states(mid, &mdef.states, None, &context);
        let initial = match &mdef.initial {
            None => top.first().copied(),
            Some(initial) => match self.lookup_state(mid, initial) {
                Ok(id) => Some(id),
                Err(kind) => {
                    self.push(kind, initial.span);
                    None
                }
            },
        };
        if let Some(initial) = initial {
            if self.model.state(initial).is_history() {
                self.push(
                    DiagnosticKind::InitialIsHistory { state: self.model.state(initial).path.clone() },
                    mdef.initial.as_ref().map_or(name.span, |i| i.span),
                );
            }
            self.model.machines[mid.index()].initial = initial;
        }
        self.model.machines[mid.index()].top_states = top;
    }

    /// Add `defs` (siblings) and their descendants in pre-order; returns the
    /// sibling ids.
    fn add_states(
        &mut self,
        machine: MachineId,
        defs: &[StateDef],
        parent: Option<StateId>,
        context: &str,
    ) -> Vec<StateId> {
        let mut seen = HashSet::new();
        let mut ids = Vec::with_capacity(defs.len());
        for def in defs {
            if !seen.insert(def.name.value.as_str()) {
                self.duplicate(context, "state", &def.name);
                continue;
            }
            ids.push(self.add_state(machine, def, parent));
        }
        ids
    }

    fn add_state(&mut self, machine: MachineId, def: &StateDef, parent: Option<StateId>) -> StateId {
        let (path, depth) = match parent {
            Some(p) => {
                let p = self.model.state(p);
                (format!("{}.{}", p.path, def.name.value), p.depth + 1)
            }
            None => (def.name.value.clone(), 0),
        };
        let id = StateId::new(self.model.states.len());
        self.model.states.push(State {
            machine,
            name: def.name.value.clone(),
            path: path.clone(),
            parent,
            kind: StateKind::Atomic,
            depth,
            span: def.name.span,
        });
        self.model.machines[machine.index()].states.push(id);

        let kind = match def.kind.value {
            StateKindDef::Normal => {
                let context = format!("state `{path}`");
                let children = self.add_states(machine, &def.states, Some(id), &context);
                self.compound_kind(id, &path, def, children)
            }
            StateKindDef::Final | StateKindDef::History | StateKindDef::DeepHistory => {
                if !def.states.is_empty() {
                    self.push(
                        DiagnosticKind::ChildrenNotAllowed { state: path.clone(), kind: def.kind.value.name() },
                        def.name.span,
                    );
                }
                if let Some(initial) = &def.initial {
                    self.push(
                        DiagnosticKind::InitialNotChild { parent: path, initial: initial.value.clone() },
                        initial.span,
                    );
                }
                match def.kind.value {
                    StateKindDef::Final => StateKind::Final,
                    StateKindDef::History => StateKind::History { deep: false },
                    StateKindDef::Normal | StateKindDef::DeepHistory => StateKind::History { deep: true },
                }
            }
        };
        self.model.states[id.index()].kind = kind;
        id
    }

    fn compound_kind(&mut self, id: StateId, path: &str, def: &StateDef, children: Vec<StateId>) -> StateKind {
        let Some(&first) = children.first() else {
            if let Some(initial) = &def.initial {
                self.push(
                    DiagnosticKind::InitialNotChild { parent: path.to_owned(), initial: initial.value.clone() },
                    initial.span,
                );
            }
            return StateKind::Atomic;
        };
        let initial = match &def.initial {
            None => first,
            Some(name) => {
                let found = children.iter().copied().find(|&c| self.model.state(c).name == name.value);
                match found {
                    Some(c) => c,
                    None => {
                        self.push(
                            DiagnosticKind::InitialNotChild { parent: path.to_owned(), initial: name.value.clone() },
                            name.span,
                        );
                        first
                    }
                }
            }
        };
        if self.model.state(initial).is_history() {
            self.push(
                DiagnosticKind::InitialIsHistory { state: self.model.state(initial).path.clone() },
                def.initial.as_ref().map_or(self.model.state(id).span, |i| i.span),
            );
        }
        StateKind::Compound { children, initial }
    }

    /// A state by full path, or by local name when that is unique.
    fn lookup_state(&self, machine: MachineId, name: &Spanned<String>) -> Result<StateId, DiagnosticKind> {
        if let Some(id) = self.model.state_by_path(machine, &name.value) {
            return Ok(id);
        }
        let matches: Vec<StateId> = self
            .model
            .machine(machine)
            .states
            .iter()
            .copied()
            .filter(|&s| self.model.state(s).name == name.value)
            .collect();
        match matches.as_slice() {
            [one] => Ok(*one),
            [] => Err(DiagnosticKind::UnknownState {
                machine: self.model.machine(machine).name.clone(),
                name: name.value.clone(),
            }),
            many => Err(DiagnosticKind::AmbiguousState {
                machine: self.model.machine(machine).name.clone(),
                name: name.value.clone(),
                candidates: many.iter().map(|&s| self.model.state(s).path.clone()).collect(),
            }),
        }
    }

    // --- Triggers and events --------------------------------------------------

    fn trigger_for(&mut self, machine: MachineId, name: &str, span: SourceSpan) -> TriggerId {
        if let Some(id) = self.model.trigger_by_name(machine, name) {
            return id;
        }
        let id = TriggerId::new(self.model.triggers.len());
        self.model.triggers.push(Trigger {
            machine,
            name: name.to_owned(),
            accepted_by: Vec::new(),
            fired_by: Vec::new(),
            sources: Vec::new(),
            span,
        });
        self.model.machines[machine.index()].triggers.push(id);
        id
    }

    fn event_for(&mut self, name: &Spanned<String>) -> Option<EventId> {
        if let Some(id) = self.model.event_by_name(&name.value) {
            return Some(id);
        }
        if self.strict_events {
            self.push(DiagnosticKind::UndeclaredEvent { event: name.value.clone() }, name.span);
            return None;
        }
        let id = EventId::new(self.model.events.len());
        self.model.event_names.insert(name.value.clone(), id);
        self.model.events.push(Event {
            name: name.value.clone(),
            payload: Vec::new(),
            declared: false,
            emitted_by: Vec::new(),
            handlers: Vec::new(),
            span: name.span,
        });
        Some(id)
    }

    fn machine_ref(&mut self, name: &str, span: SourceSpan) -> Option<MachineId> {
        let found = self.model.machine_by_name(name);
        if found.is_none() && !self.failed_machines.contains(name) {
            self.push(DiagnosticKind::UnknownMachine { name: name.to_owned() }, span);
        }
        found
    }

    // --- Transitions ------------------------------------------------------------

    fn add_transitions(&mut self, mid: MachineId, mdef: &MachineDef) {
        for tdef in &mdef.transitions {
            let to = self.lookup_state(mid, &tdef.to);
            let trigger = self.trigger_for(mid, &tdef.on.value, tdef.on.span);
            let mut emits = Vec::new();
            for event in &tdef.emits {
                if let Some(e) = self.event_for(event)
                    && !emits.contains(&e)
                {
                    emits.push(e);
                }
            }
            let to = match to {
                Ok(to) => Some(to),
                Err(kind) => {
                    self.push(kind, tdef.to.span);
                    None
                }
            };
            for from_name in &tdef.from {
                let from = match self.lookup_state(mid, from_name) {
                    Ok(from) => from,
                    Err(kind) => {
                        self.push(kind, from_name.span);
                        continue;
                    }
                };
                let from_state = self.model.state(from);
                if from_state.is_final() {
                    self.push(DiagnosticKind::TransitionFromFinal { state: from_state.path.clone() }, from_name.span);
                    continue;
                }
                if from_state.is_history() {
                    self.push(DiagnosticKind::TransitionFromHistory { state: from_state.path.clone() }, from_name.span);
                    continue;
                }
                let Some(to) = to else { continue };
                let ordinal = self.model.machines[mid.index()]
                    .transitions
                    .iter()
                    .filter(|&&t| {
                        let t = &self.model.transitions[t.index()];
                        t.from == from && t.to == to && t.trigger == trigger
                    })
                    .count();
                let id = TransitionId::new(self.model.transitions.len());
                self.model.transitions.push(Transition {
                    machine: mid,
                    from,
                    to,
                    trigger,
                    guard: tdef.guard.as_ref().map(|g| g.value.clone()),
                    emits: emits.clone(),
                    bounded: tdef.bounded,
                    ordinal: u32::try_from(ordinal).unwrap_or(u32::MAX),
                    span: tdef.span,
                });
                self.model.machines[mid.index()].transitions.push(id);
                self.model.triggers[trigger.index()].accepted_by.push(id);
                for &e in &emits {
                    self.model.events[e.index()].emitted_by.push(id);
                }
            }
        }
    }

    // --- Controllers ------------------------------------------------------------

    fn add_controllers(&mut self, definition: &Definition) {
        for cdef in &definition.controllers {
            if self.model.controller_names.contains_key(&cdef.name.value) {
                self.duplicate("controllers", "controller", &cdef.name);
                continue;
            }
            let cid = ControllerId::new(self.model.controllers.len());
            self.model.controller_names.insert(cdef.name.value.clone(), cid);
            self.model.controllers.push(Controller {
                name: cdef.name.value.clone(),
                handlers: Vec::new(),
                span: cdef.name.span,
            });

            for hdef in &cdef.on {
                let Some(event) = self.event_for(&hdef.event) else {
                    continue;
                };
                if self.model.handler_for(cid, event).is_some() {
                    self.duplicate(&format!("controller `{}`", cdef.name.value), "subscription", &hdef.event);
                    continue;
                }
                let hid = HandlerId::new(self.model.handlers.len());
                self.model.handlers.push(Handler { controller: cid, event, rules: Vec::new(), span: hdef.span });
                self.model.controllers[cid.index()].handlers.push(hid);
                self.model.events[event.index()].handlers.push(hid);

                for (ordinal, rdef) in hdef.rules.iter().enumerate() {
                    let fire = &rdef.fire;
                    let Some(machine) = self.machine_ref(&fire.value.machine, fire.span) else {
                        continue;
                    };
                    let Some(target) = self.resolve_target(machine, event, rdef) else {
                        continue;
                    };
                    let trigger = self.trigger_for(machine, &fire.value.trigger, fire.span);
                    let rid = RuleId::new(self.model.rules.len());
                    self.model.rules.push(Rule {
                        controller: cid,
                        handler: hid,
                        event,
                        trigger,
                        target,
                        condition: rdef.when.as_ref().map(|w| w.value.clone()),
                        bounded: rdef.bounded,
                        ordinal: u32::try_from(ordinal).unwrap_or(u32::MAX),
                        span: rdef.span,
                    });
                    self.model.handlers[hid.index()].rules.push(rid);
                    self.model.triggers[trigger.index()].fired_by.push(rid);
                }
            }
        }
    }

    fn resolve_target(
        &mut self,
        machine: MachineId,
        event: EventId,
        rdef: &crate::definition::RuleDef,
    ) -> Option<Target> {
        let Some(spec) = &rdef.target else {
            return Some(Target::One { predicates: Vec::new() });
        };
        if spec.value.machine != rdef.fire.value.machine {
            self.push(
                DiagnosticKind::TargetMachineMismatch {
                    fire: rdef.fire.value.to_string(),
                    target: spec.value.machine.clone(),
                },
                spec.span,
            );
            return None;
        }
        let mut ok = true;
        for clause in &spec.value.clauses {
            ok &= self.check_clause(machine, event, clause, spec.span);
        }
        if !ok {
            return None;
        }
        let clauses = spec.value.clauses.clone();
        Some(match spec.value.mode {
            TargetMode::One => Target::One { predicates: clauses },
            TargetMode::All => Target::All { predicates: clauses },
            TargetMode::Spawn => Target::Spawn { assignments: clauses },
        })
    }

    fn check_clause(&mut self, machine: MachineId, event: EventId, clause: &FieldClause, span: SourceSpan) -> bool {
        let mut ok = true;
        let machine = self.model.machine(machine);
        if !machine.fields.is_empty() && !machine.fields.contains(&clause.field) {
            let kind = DiagnosticKind::UnknownField {
                machine: machine.name.clone(),
                field: clause.field.clone(),
                declared: machine.fields.clone(),
            };
            self.push(kind, span);
            ok = false;
        }
        if let ValueExpr::EventField(field) = &clause.value {
            let event = self.model.event(event);
            if event.declared && !event.payload.is_empty() && !event.payload.contains(field) {
                let kind = DiagnosticKind::UnknownPayloadField {
                    event: event.name.clone(),
                    field: field.clone(),
                    declared: event.payload.clone(),
                };
                self.push(kind, span);
                ok = false;
            }
        }
        ok
    }

    // --- External sources ---------------------------------------------------------

    fn add_externals(&mut self, definition: &Definition) {
        for edef in &definition.external {
            if self.model.external_names.contains_key(&edef.name.value) {
                self.duplicate("external", "external source", &edef.name);
                continue;
            }
            let xid = ExternalId::new(self.model.externals.len());
            self.model.external_names.insert(edef.name.value.clone(), xid);
            let mut triggers = Vec::new();
            for tref in &edef.triggers {
                let Some(machine) = self.machine_ref(&tref.value.machine, tref.span) else {
                    continue;
                };
                let trigger = self.trigger_for(machine, &tref.value.trigger, tref.span);
                if !triggers.contains(&trigger) {
                    triggers.push(trigger);
                    self.model.triggers[trigger.index()].sources.push(xid);
                }
            }
            self.model.externals.push(ExternalSource { name: edef.name.value.clone(), triggers, span: edef.name.span });
        }
    }
}
