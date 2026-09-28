//! Event declarations and event renames.
//!
//! A file with no `events:` is lenient: events exist by being emitted or
//! subscribed to. Declaring the first event switches it to strict mode, so
//! every event already in use is declared along with it (payload unknown).
//! Removing a declaration is left to the resolver to judge: it fails while
//! a strict file still uses the event, and always succeeds for the last
//! declaration, which switches the file back to lenient mode.

use super::engine::{self, Effect, InverseMode, push_unique};
use super::keys::{self, event_key};
use super::lookup::{self, name_taken, not_found};
use super::{EditError, EditOp, Index, validate};
use crate::definition::{Definition, EventDef};
use crate::span::{SourceSpan, Spanned};

/// Every event the definition mentions, declared ones first, then in order
/// of first mention: transition emits by machine, then subscriptions by
/// controller.
fn known_events(def: &Definition) -> Vec<String> {
    let mut out: Vec<String> = def.events.iter().map(|e| e.name.value.clone()).collect();
    let emitted = def.machines.iter().flat_map(|m| m.transitions.iter().flat_map(|t| t.emits.iter()));
    let handled = def.controllers.iter().flat_map(|c| c.on.iter().map(|h| &h.event));
    for name in emitted.chain(handled) {
        if !out.contains(&name.value) {
            out.push(name.value.clone());
        }
    }
    out
}

pub(super) fn declare(def: &mut Definition, event: &EventDef, index: Index) -> Result<Effect, EditError> {
    validate::event(event)?;
    let name = event.name.value.clone();
    if def.events.iter().any(|e| e.name.value == name) {
        return Err(name_taken("event", name));
    }
    if !def.events.is_empty() {
        lookup::insert(&mut def.events, event.clone(), index, "event")?;
        return Ok(Effect::new(EditOp::RemoveEventDeclaration { event: name.clone() }, vec![event_key(&name)]));
    }

    // Lenient → strict: declare everything in use, then place this one.
    let mut declared: Vec<EventDef> = known_events(def)
        .into_iter()
        .filter(|used| *used != name)
        .map(|used| EventDef { name: Spanned::synthetic(used), payload: Vec::new(), span: SourceSpan::unknown() })
        .collect();
    lookup::insert(&mut declared, event.clone(), index, "event")?;
    let touched: Vec<_> = declared.iter().map(|e| event_key(&e.name.value)).collect();
    let inverse =
        declared.iter().rev().map(|e| EditOp::RemoveEventDeclaration { event: e.name.value.clone() }).collect();
    def.events = declared;
    Ok(Effect::new(engine::sequence(inverse), touched))
}

pub(super) fn remove_declaration(def: &mut Definition, event: &str, mode: InverseMode) -> Result<Effect, EditError> {
    let at =
        def.events.iter().position(|e| e.name.value == event).ok_or_else(|| not_found("event declaration", event))?;
    let original: Vec<String> = def.events.iter().map(|e| e.name.value.clone()).collect();
    let removed = def.events.remove(at);
    let mut inverse = vec![EditOp::DeclareEvent { event: removed, index: Some(at) }];

    // Re-declaring into an empty list declares everything in use; when that
    // is more than was declared before (a batch removing several strict
    // declarations), take the extras out again.
    if def.events.is_empty() && mode == InverseMode::Exact {
        let simulated = engine::simulate(def, &inverse)?;
        let extras: Vec<EditOp> = simulated
            .events
            .iter()
            .filter(|e| !original.contains(&e.name.value))
            .map(|e| EditOp::RemoveEventDeclaration { event: e.name.value.clone() })
            .collect();
        inverse.extend(extras);
    }
    Ok(Effect::new(engine::sequence(inverse), vec![event_key(event)]))
}

/// Renames the event in its declaration, every `emits:` and every
/// subscription.
pub(super) fn rename(def: &mut Definition, from: &str, to: &str) -> Result<Effect, EditError> {
    validate::name(to)?;
    let known = known_events(def);
    if !known.iter().any(|e| e == from) {
        return Err(not_found("event", from));
    }
    let inverse = EditOp::RenameEvent { from: to.to_owned(), to: from.to_owned() };
    if from == to {
        return Ok(Effect::new(inverse, vec![event_key(from)]));
    }
    if known.iter().any(|e| e == to) {
        return Err(name_taken("event", to));
    }

    let mut touched = vec![event_key(to)];
    for e in def.events.iter_mut().filter(|e| e.name.value == from) {
        lookup::set_value(&mut e.name, to.to_owned());
    }
    for m in &mut def.machines {
        let mut entries = Vec::new();
        for (i, t) in m.transitions.iter_mut().enumerate() {
            for emit in t.emits.iter_mut().filter(|e| e.value == from) {
                lookup::set_value(emit, to.to_owned());
                entries.push(i);
            }
        }
        push_unique(&mut touched, keys::entries_keys(m, &entries));
    }
    for c in &mut def.controllers {
        for h in c.on.iter_mut().filter(|h| h.event.value == from) {
            lookup::set_value(&mut h.event, to.to_owned());
            push_unique(&mut touched, keys::handler_keys(&c.name.value, h));
        }
    }
    Ok(Effect::new(inverse, touched))
}
