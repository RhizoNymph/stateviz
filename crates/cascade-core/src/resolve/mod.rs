//! [`Definition`] → [`Model`]: resolve every name to a typed id, build the
//! statechart structure and the reverse indexes, and report every dangling
//! or contradictory reference.

mod typing;

use std::collections::{HashMap, HashSet};

use crate::definition::{
    Definition, EmitDef, FieldDef, MachineDef, RuleDef, StateDef, StateKindDef, TargetSpec, TransitionDef, TriggerSpec,
    timer_trigger_name,
};
use crate::error::{Diagnostic, DiagnosticKind, LoadError};
use crate::expr::{Assignment, Condition, Expr, FieldDecl, Guard};
use crate::ids::{ControllerId, EventId, ExternalId, HandlerId, MachineId, RuleId, StateId, TransitionId, TriggerId};
use crate::model::{
    Controller, Emit, Event, ExternalSource, Handler, Machine, Model, Rule, State, StateKind, Target, Transition,
    Trigger, TriggerKind,
};
use crate::span::{SourceSpan, Spanned};
use typing::{EventScope, SelfScope, Site};

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
        let payload = r.field_decls(&edef.payload, &format!("event `{}` payload", edef.name.value));
        r.model.events.push(Event {
            name: edef.name.value.clone(),
            payload,
            declared: true,
            emitted_by: Vec::new(),
            handlers: Vec::new(),
            subscribers: Vec::new(),
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

    fn push_all(&mut self, kinds: Vec<DiagnosticKind>, span: SourceSpan) {
        for kind in kinds {
            self.push(kind, span);
        }
    }

    /// Declarations of a field list, reporting duplicate names.
    fn field_decls(&mut self, fields: &[FieldDef], context: &str) -> Vec<FieldDecl> {
        let mut out: Vec<FieldDecl> = Vec::with_capacity(fields.len());
        for field in fields {
            if out.iter().any(|f| f.name == field.name.value) {
                self.duplicate(context, "field", &field.name);
            } else {
                out.push(field.decl());
            }
        }
        out
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
        let fields = self.field_decls(&mdef.fields, &format!("machine `{}` fields", name.value));
        // Push a placeholder so states can refer to the machine id; the real
        // initial state is set once the states exist.
        self.model.machines.push(Machine {
            name: name.value.clone(),
            color: mdef.color.as_ref().map(|c| c.value),
            domain: mdef.domain.as_ref().map(|d| d.value.clone()),
            initial: StateId::new(self.model.states.len()),
            fields,
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

    /// The trigger named `name` on `machine`, created with `kind` on first
    /// mention. A name already used with another kind is a conflict (only
    /// commands may be fired by rules and external sources).
    fn trigger_for(
        &mut self,
        machine: MachineId,
        name: &str,
        kind: TriggerKind,
        span: SourceSpan,
    ) -> Option<TriggerId> {
        if let Some(id) = self.model.trigger_by_name(machine, name) {
            let existing = self.model.trigger(id).kind;
            if existing == kind {
                return Some(id);
            }
            self.push(
                DiagnosticKind::TriggerKindConflict {
                    machine: self.model.machine(machine).name.clone(),
                    trigger: name.to_owned(),
                    first: existing.describe(),
                    second: kind.describe(),
                },
                span,
            );
            return None;
        }
        let id = TriggerId::new(self.model.triggers.len());
        self.model.triggers.push(Trigger {
            machine,
            name: name.to_owned(),
            kind,
            accepted_by: Vec::new(),
            fired_by: Vec::new(),
            sources: Vec::new(),
            span,
        });
        self.model.machines[machine.index()].triggers.push(id);
        if let TriggerKind::Event(event) = kind {
            self.model.events[event.index()].subscribers.push(id);
        }
        Some(id)
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
            subscribers: Vec::new(),
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
            let trigger = self.transition_trigger(mid, &tdef.trigger);
            let event = trigger.and_then(|t| match self.model.trigger(t).kind {
                TriggerKind::Event(e) => Some(e),
                TriggerKind::Command | TriggerKind::Timer(_) => None,
            });
            let checked = self.check_transition(mid, event, tdef);
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
                let (Some(to), Some(trigger)) = (to, trigger) else { continue };
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
                    guard: checked.guard.clone(),
                    filter: checked.filter.clone(),
                    sets: checked.sets.clone(),
                    emits: checked.emits.clone(),
                    bounded: tdef.bounded,
                    ordinal: u32::try_from(ordinal).unwrap_or(u32::MAX),
                    span: tdef.span,
                });
                self.model.machines[mid.index()].transitions.push(id);
                self.model.triggers[trigger.index()].accepted_by.push(id);
                for emit in &checked.emits {
                    self.model.events[emit.event.index()].emitted_by.push(id);
                }
            }
        }
    }

    fn transition_trigger(&mut self, mid: MachineId, spec: &TriggerSpec) -> Option<TriggerId> {
        match spec {
            TriggerSpec::Command(name) => self.trigger_for(mid, &name.value, TriggerKind::Command, name.span),
            TriggerSpec::Event { event, .. } => {
                let e = self.event_for(event)?;
                self.trigger_for(mid, &event.value, TriggerKind::Event(e), event.span)
            }
            TriggerSpec::After(delay) if delay.value.is_zero() => {
                self.push(
                    DiagnosticKind::InvalidDuration {
                        text: delay.value.to_string(),
                        message: "a timer's delay must be greater than zero".to_owned(),
                    },
                    delay.span,
                );
                None
            }
            TriggerSpec::After(delay) => {
                self.trigger_for(mid, &timer_trigger_name(delay.value), TriggerKind::Timer(delay.value), delay.span)
            }
        }
    }

    /// Type-check a transition's guard, filter, sets and emits; `event` is
    /// the subscribed event (the payload in scope), if any.
    fn check_transition(&mut self, mid: MachineId, event: Option<EventId>, tdef: &TransitionDef) -> CheckedTransition {
        let machine = self.model.machine(mid).clone();
        let event_def = event.map(|e| self.model.event(e).clone());
        let site = Site {
            self_scope: SelfScope::Instance(&machine),
            event: event_def.as_ref().map_or(EventScope::Open, EventScope::Known),
        };
        let mut diags = Vec::new();

        if let Some(guard) = &tdef.guard
            && let Guard::Expr(expr) = &guard.value
        {
            diags.push((site.check_expr(expr, &guard.value.to_string()), guard.span));
        }
        let filter = match &tdef.trigger {
            TriggerSpec::Event { filter: Some(filter), .. } => {
                diags.push((site.check_expr(&filter.value, &filter.value.display_filter().to_string()), filter.span));
                Some(filter.value.clone())
            }
            TriggerSpec::Event { filter: None, .. } | TriggerSpec::Command(_) | TriggerSpec::After(_) => None,
        };
        for set in &tdef.sets {
            let mut found = Vec::new();
            let target = match machine.fields.iter().find(|f| f.name == set.value.field) {
                Some(f) => Some(&f.ty),
                None if machine.fields.is_empty() => None,
                None => {
                    found.push(typing::unknown_field(&machine, &set.value.field));
                    None
                }
            };
            found.extend(site.check_value(target, &set.value));
            diags.push((found, set.span));
        }
        let mut emits: Vec<Emit> = Vec::new();
        for edef in &tdef.emits {
            let Some(emitted) = self.event_for(&edef.event) else { continue };
            diags.extend(self.check_with(&site, emitted, edef));
            if !emits.iter().any(|e| e.event == emitted) {
                emits.push(Emit { event: emitted, with: edef.with.iter().map(|w| w.value.clone()).collect() });
            }
        }
        for (kinds, span) in diags {
            self.push_all(kinds, span);
        }
        CheckedTransition {
            guard: tdef.guard.as_ref().map(|g| g.value.clone()),
            filter,
            sets: tdef.sets.iter().map(|s| s.value.clone()).collect(),
            emits,
        }
    }

    /// `with:` keys must be payload fields of the emitted event (when it
    /// declares a payload) and values must fit them.
    fn check_with(&self, site: &Site<'_>, emitted: EventId, edef: &EmitDef) -> Vec<(Vec<DiagnosticKind>, SourceSpan)> {
        let event = self.model.event(emitted);
        let checks_payload = event.declared && !event.payload.is_empty();
        edef.with
            .iter()
            .map(|w| {
                let mut found = Vec::new();
                let target = match event.payload.iter().find(|f| f.name == w.value.field) {
                    Some(f) => Some(&f.ty),
                    None => {
                        if checks_payload {
                            found.push(typing::unknown_payload_field(event, &w.value.field));
                        }
                        None
                    }
                };
                found.extend(site.check_value(target, &w.value));
                (found, w.span)
            })
            .collect()
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
                    self.check_condition(event, rdef);
                    let Some(machine) = self.machine_ref(&fire.value.machine, fire.span) else {
                        continue;
                    };
                    let Some(target) = self.resolve_target(machine, event, rdef) else {
                        continue;
                    };
                    let Some(trigger) = self.trigger_for(machine, &fire.value.trigger, TriggerKind::Command, fire.span)
                    else {
                        continue;
                    };
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

    /// A rule's `when:` reads the handled event; there is no instance.
    fn check_condition(&mut self, event: EventId, rdef: &RuleDef) {
        let Some(when) = &rdef.when else { return };
        let Condition::Expr(expr) = &when.value else { return };
        let event = self.model.event(event).clone();
        let site = Site { self_scope: SelfScope::None("a rule's `when`"), event: EventScope::Known(&event) };
        let found = site.check_expr(expr, &when.value.to_string());
        self.push_all(found, when.span);
    }

    fn resolve_target(&mut self, machine: MachineId, event: EventId, rdef: &RuleDef) -> Option<Target> {
        let Some(spec) = &rdef.target else {
            return Some(Target::One { filter: None });
        };
        if spec.value.machine() != rdef.fire.value.machine {
            self.push(
                DiagnosticKind::TargetMachineMismatch {
                    fire: rdef.fire.value.to_string(),
                    target: spec.value.machine().to_owned(),
                },
                spec.span,
            );
            return None;
        }
        let machine_def = self.model.machine(machine).clone();
        let event_def = self.model.event(event).clone();
        let found = match &spec.value {
            TargetSpec::One { filter: Some(filter), .. } | TargetSpec::All { filter: Some(filter), .. } => {
                let site = Site { self_scope: SelfScope::Instance(&machine_def), event: EventScope::Known(&event_def) };
                site.check_expr(filter, &filter.display_filter().to_string())
            }
            TargetSpec::One { filter: None, .. } | TargetSpec::All { filter: None, .. } => Vec::new(),
            TargetSpec::Spawn { assignments, .. } => {
                let site =
                    Site { self_scope: SelfScope::None("a spawn assignment"), event: EventScope::Known(&event_def) };
                assignments.iter().flat_map(|a| check_spawn(&site, &machine_def, a)).collect()
            }
        };
        let ok = found.is_empty();
        self.push_all(found, spec.span);
        if !ok {
            return None;
        }
        Some(match &spec.value {
            TargetSpec::One { filter, .. } => Target::One { filter: filter.clone() },
            TargetSpec::All { filter, .. } => Target::All { filter: filter.clone() },
            TargetSpec::Spawn { assignments, .. } => Target::Spawn { assignments: assignments.clone() },
        })
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
                let Some(trigger) = self.trigger_for(machine, &tref.value.trigger, TriggerKind::Command, tref.span)
                else {
                    continue;
                };
                if !triggers.contains(&trigger) {
                    triggers.push(trigger);
                    self.model.triggers[trigger.index()].sources.push(xid);
                }
            }
            self.model.externals.push(ExternalSource { name: edef.name.value.clone(), triggers, span: edef.name.span });
        }
    }
}

/// A transition's checked parts, shared by its expansions.
struct CheckedTransition {
    guard: Option<Guard>,
    filter: Option<Expr>,
    sets: Vec<Assignment>,
    emits: Vec<Emit>,
}

/// A spawn assignment names a field of the spawned machine (when it
/// declares fields) and takes a value from the handled event or a literal.
fn check_spawn(site: &Site<'_>, machine: &Machine, assignment: &Assignment) -> Vec<DiagnosticKind> {
    let mut found = Vec::new();
    let target = match machine.fields.iter().find(|f| f.name == assignment.field) {
        Some(f) => Some(&f.ty),
        None if machine.fields.is_empty() => None,
        None => {
            found.push(typing::unknown_field(machine, &assignment.field));
            None
        }
    };
    found.extend(site.check_value(target, assignment));
    found
}
