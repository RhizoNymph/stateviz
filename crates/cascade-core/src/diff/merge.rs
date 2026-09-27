//! The union definition behind [`merge_for_display`](super::merge_for_display):
//! the new definition plus ghosts of the removed elements, arranged so the
//! union resolves.

use std::collections::{HashMap, HashSet};

use crate::definition::{ControllerDef, Definition, MachineDef, StateDef, StateKindDef, ValueExpr};
use crate::ids::{ControllerId, MachineId, RuleId, StateId};
use crate::key::{ElementKey, ElementRef};
use crate::model::{Model, StateKind, Target};
use crate::span::{SourceSpan, Spanned};

use super::rebuild;

/// The new definition with the removed elements of `old` added back.
pub(super) fn union_definition(old: &Model, new: &Model) -> Definition {
    let mut union = new.definition().clone();
    canonicalize(&mut union, new);
    let mut needs = SelectorNeeds::default();
    merge_machines(&mut union, old, new);
    merge_controllers(&mut union, old, &mut needs);
    merge_externals(&mut union, old);
    declare_removed_events(&mut union, old);
    widen_checked_lists(&mut union, &needs);
    union
}

/// Give every element of the merged model the span it has in `new`, and
/// ghosts no span. Resolution records the *first mention* of triggers and
/// undeclared events, which in the union can be a ghost.
pub(super) fn restore_spans(merged: &mut Model, new: &Model) {
    for element in merged.all_elements() {
        let span = new.resolve_key(&merged.key_of(element)).map_or(SourceSpan::unknown(), |e| new.span_of(e));
        set_span(merged, element, span);
    }
}

fn set_span(model: &mut Model, element: ElementRef, span: SourceSpan) {
    let slot = match element {
        ElementRef::Machine(id) => model.machines.get_mut(id.index()).map(|x| &mut x.span),
        ElementRef::State(id) => model.states.get_mut(id.index()).map(|x| &mut x.span),
        ElementRef::Transition(id) => model.transitions.get_mut(id.index()).map(|x| &mut x.span),
        ElementRef::Trigger(id) => model.triggers.get_mut(id.index()).map(|x| &mut x.span),
        ElementRef::Event(id) => model.events.get_mut(id.index()).map(|x| &mut x.span),
        ElementRef::Controller(id) => model.controllers.get_mut(id.index()).map(|x| &mut x.span),
        ElementRef::Handler(id) => model.handlers.get_mut(id.index()).map(|x| &mut x.span),
        ElementRef::Rule(id) => model.rules.get_mut(id.index()).map(|x| &mut x.span),
        ElementRef::External(id) => model.externals.get_mut(id.index()).map(|x| &mut x.span),
    };
    if let Some(slot) = slot {
        *slot = span;
    }
}

// --- Canonical references -------------------------------------------------------

/// Rewrite the new definition's state references to full paths and make
/// every machine's initial state explicit. Ghost states can turn a unique
/// local name ambiguous, and a ghost inserted first would otherwise become
/// the default initial state.
fn canonicalize(def: &mut Definition, new: &Model) {
    for mdef in &mut def.machines {
        let Some(id) = new.machine_by_name(&mdef.name.value) else { continue };
        let initial = new.state(new.machine(id).initial).path.clone();
        let span = mdef.initial.as_ref().map_or(SourceSpan::unknown(), |i| i.span);
        mdef.initial = Some(Spanned::new(initial, span));
        for tdef in &mut mdef.transitions {
            for from in &mut tdef.from {
                canonical_path(new, id, &mut from.value);
            }
            canonical_path(new, id, &mut tdef.to.value);
        }
    }
}

fn canonical_path(model: &Model, machine: MachineId, reference: &mut String) {
    if let Some(state) = lookup_state(model, machine, reference) {
        reference.clone_from(&model.state(state).path);
    }
}

/// A state by full path, or by local name when that is unique (the
/// resolver's rule).
fn lookup_state(model: &Model, machine: MachineId, name: &str) -> Option<StateId> {
    model.state_by_path(machine, name).or_else(|| {
        let mut matches = model.machine(machine).states.iter().copied().filter(|&s| model.state(s).name == name);
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    })
}

/// Insert `item` right after the last old neighbour placed so far (or
/// first), and remember where it went.
fn place<T>(items: &mut Vec<T>, cursor: &mut Option<usize>, item: T) {
    let at = cursor.map_or(0, |c| c + 1).min(items.len());
    items.insert(at, item);
    *cursor = Some(at);
}

// --- Machines, states and transitions -----------------------------------------------

fn merge_machines(union: &mut Definition, old: &Model, new: &Model) {
    let mut cursor = None;
    for (old_id, machine) in old.machines() {
        match union.machines.iter().position(|m| m.name.value == machine.name) {
            Some(ix) => {
                cursor = Some(ix);
                if let (Some(mdef), Some(new_id)) = (union.machines.get_mut(ix), new.machine_by_name(&machine.name)) {
                    merge_machine(mdef, old, old_id, new, new_id);
                }
            }
            None => place(&mut union.machines, &mut cursor, rebuild::machine_def(old, old_id)),
        }
    }
}

/// Add the removed states and transitions of a machine both versions have.
fn merge_machine(mdef: &mut MachineDef, old: &Model, old_id: MachineId, new: &Model, new_id: MachineId) {
    merge_states(&mut mdef.states, &old.machine(old_id).top_states, old);

    let kinds = state_kinds(&mdef.states);
    let present: HashSet<ElementKey> =
        new.machine(new_id).transitions.iter().map(|&t| new.key_of(ElementRef::Transition(t))).collect();
    // Appending in old order gives each ghost its old ordinal: for a
    // (from, to, trigger) triple the new version keeps ordinals
    // 0..n_new and the removed ones are exactly n_new..n_old.
    for &t in &old.machine(old_id).transitions {
        if present.contains(&old.key_of(ElementRef::Transition(t))) {
            continue;
        }
        let transition = old.transition(t);
        let leaves_normal_state = kinds.get(&old.state(transition.from).path) == Some(&StateKindDef::Normal);
        let target_exists = kinds.contains_key(&old.state(transition.to).path);
        if leaves_normal_state && target_exists {
            mdef.transitions.push(rebuild::transition_def(old, t));
        }
    }
}

/// Insert the old states missing from `siblings`, each after its nearest
/// preceding old sibling, and recurse into the states both versions have.
/// Returns whether a ghost was inserted at this level.
fn merge_states(siblings: &mut Vec<StateDef>, old_states: &[StateId], old: &Model) -> bool {
    let mut cursor = None;
    let mut inserted = false;
    for &id in old_states {
        let state = old.state(id);
        match siblings.iter().position(|s| s.name.value == state.name) {
            Some(ix) => {
                cursor = Some(ix);
                let Some(sibling) = siblings.get_mut(ix) else { continue };
                // Final and history states cannot have children: their
                // removed children have no ghost.
                if sibling.kind.value != StateKindDef::Normal || state.children().is_empty() {
                    continue;
                }
                let default_initial = sibling.states.first().map(|s| s.name.value.clone());
                if merge_states(&mut sibling.states, state.children(), old) && sibling.initial.is_none() {
                    // Keep the new default child; a state that was atomic in
                    // the new version gets the old initial child (never a
                    // history state).
                    let initial = default_initial.or_else(|| match &state.kind {
                        StateKind::Compound { initial, .. } => Some(old.state(*initial).name.clone()),
                        StateKind::Atomic | StateKind::Final | StateKind::History { .. } => None,
                    });
                    sibling.initial = initial.map(Spanned::synthetic);
                }
            }
            None => {
                place(siblings, &mut cursor, rebuild::state_def(old, id));
                inserted = true;
            }
        }
    }
    inserted
}

/// Every state path in a state tree, with its kind.
fn state_kinds(states: &[StateDef]) -> HashMap<String, StateKindDef> {
    let mut out = HashMap::new();
    collect_kinds(states, None, &mut out);
    out
}

fn collect_kinds(states: &[StateDef], prefix: Option<&str>, out: &mut HashMap<String, StateKindDef>) {
    for state in states {
        let path = match prefix {
            Some(parent) => format!("{parent}.{}", state.name.value),
            None => state.name.value.clone(),
        };
        collect_kinds(&state.states, Some(&path), out);
        out.insert(path, state.kind.value);
    }
}

// --- Controllers, sources and events --------------------------------------------------

/// Fields and payload fields that ghost rule selectors are checked against.
#[derive(Default)]
struct SelectorNeeds {
    /// Machine name → fields named by ghost selectors firing into it.
    machine_fields: HashMap<String, Vec<String>>,
    /// Event name → payload fields ghost selectors read (`event.x`).
    event_fields: HashMap<String, Vec<String>>,
}

impl SelectorNeeds {
    fn note(&mut self, model: &Model, rule: RuleId) {
        let r = model.rule(rule);
        let clauses = match &r.target {
            Target::One { predicates } | Target::All { predicates } => predicates,
            Target::Spawn { assignments } => assignments,
        };
        let machine = &model.machine(model.trigger(r.trigger).machine).name;
        let event = &model.event(r.event).name;
        for clause in clauses {
            push_unique(self.machine_fields.entry(machine.clone()).or_default(), &clause.field);
            if let ValueExpr::EventField(field) = &clause.value {
                push_unique(self.event_fields.entry(event.clone()).or_default(), field);
            }
        }
    }
}

fn push_unique(list: &mut Vec<String>, item: &str) {
    if !list.iter().any(|x| x == item) {
        list.push(item.to_owned());
    }
}

fn merge_controllers(union: &mut Definition, old: &Model, needs: &mut SelectorNeeds) {
    let mut cursor = None;
    for (old_id, controller) in old.controllers() {
        match union.controllers.iter().position(|c| c.name.value == controller.name) {
            Some(ix) => {
                cursor = Some(ix);
                if let Some(cdef) = union.controllers.get_mut(ix) {
                    merge_handlers(cdef, old, old_id, needs);
                }
            }
            None => {
                for &h in &controller.handlers {
                    for &r in &old.handler(h).rules {
                        needs.note(old, r);
                    }
                }
                place(&mut union.controllers, &mut cursor, rebuild::controller_def(old, old_id));
            }
        }
    }
}

/// Add the removed handlers and rules of a controller both versions have.
/// A rule's ordinal is its position, so the removed rules of a kept handler
/// are the old ones past the new rule count, appended in order.
fn merge_handlers(cdef: &mut ControllerDef, old: &Model, old_id: ControllerId, needs: &mut SelectorNeeds) {
    for &h in &old.controller(old_id).handlers {
        let handler = old.handler(h);
        let event = &old.event(handler.event).name;
        match cdef.on.iter_mut().find(|hdef| hdef.event.value == *event) {
            Some(hdef) => {
                let kept = hdef.rules.len();
                for &r in &handler.rules {
                    if usize::try_from(old.rule(r).ordinal).is_ok_and(|ordinal| ordinal >= kept) {
                        needs.note(old, r);
                        hdef.rules.push(rebuild::rule_def(old, r));
                    }
                }
            }
            None => {
                for &r in &handler.rules {
                    needs.note(old, r);
                }
                cdef.on.push(rebuild::handler_def(old, h));
            }
        }
    }
}

/// Removed sources come back whole. Sources in both versions keep only
/// their new triggers: an old trigger added to them would draw an edge that
/// exists in neither version.
fn merge_externals(union: &mut Definition, old: &Model) {
    let mut cursor = None;
    for (old_id, source) in old.externals() {
        match union.external.iter().position(|x| x.name.value == source.name) {
            Some(ix) => cursor = Some(ix),
            None => place(&mut union.external, &mut cursor, rebuild::external_def(old, old_id)),
        }
    }
}

/// When the new version declares its events (strict mode), declare every
/// removed event so the ghosts that emit or subscribe to it resolve. A
/// non-strict union gets no declarations: they would make it strict.
fn declare_removed_events(union: &mut Definition, old: &Model) {
    if union.events.is_empty() {
        return;
    }
    for (id, event) in old.events() {
        if !union.events.iter().any(|e| e.name.value == event.name) {
            union.events.push(rebuild::event_def(old, id));
        }
    }
}

/// Declared field and payload lists are checked against selectors; add what
/// ghost selectors use. Empty lists mean "unchecked" and stay empty.
fn widen_checked_lists(union: &mut Definition, needs: &SelectorNeeds) {
    for mdef in &mut union.machines {
        if let Some(fields) = needs.machine_fields.get(&mdef.name.value)
            && !mdef.fields.is_empty()
        {
            for field in fields {
                if !mdef.fields.iter().any(|f| f.value == *field) {
                    mdef.fields.push(Spanned::synthetic(field.clone()));
                }
            }
        }
    }
    for edef in &mut union.events {
        if let Some(fields) = needs.event_fields.get(&edef.name.value)
            && !edef.payload.is_empty()
        {
            for field in fields {
                if !edef.payload.iter().any(|f| f.value == *field) {
                    edef.payload.push(Spanned::synthetic(field.clone()));
                }
            }
        }
    }
}
