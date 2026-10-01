//! Adding, replacing and removing `transitions:` entries.

use super::engine::Effect;
use super::{EditError, EditOp, Index, keys, lookup, validate};
use crate::definition::{Definition, TransitionDef};

pub(super) fn add(
    def: &mut Definition,
    machine: &str,
    transition: &TransitionDef,
    index: Index,
) -> Result<Effect, EditError> {
    validate::transition(transition)?;
    let mdef = lookup::machine_mut(def, machine)?;
    let at = lookup::insert(&mut mdef.transitions, transition.clone(), index, "transition")?;
    let touched = keys::transition_keys(mdef, at);
    Ok(Effect::new(EditOp::RemoveTransition { machine: machine.to_owned(), index: at }, touched))
}

pub(super) fn update(
    def: &mut Definition,
    machine: &str,
    index: usize,
    transition: &TransitionDef,
) -> Result<Effect, EditError> {
    let mdef = lookup::machine_mut(def, machine)?;
    let at = lookup::existing(index, mdef.transitions.len(), "transition")?;
    validate::transition(transition)?;
    let old = std::mem::replace(&mut mdef.transitions[at], transition.clone());
    let touched = keys::transition_keys(mdef, at);
    Ok(Effect::new(EditOp::UpdateTransition { machine: machine.to_owned(), index: at, transition: old }, touched))
}

pub(super) fn remove(def: &mut Definition, machine: &str, index: usize) -> Result<Effect, EditError> {
    let mdef = lookup::machine_mut(def, machine)?;
    let at = lookup::existing(index, mdef.transitions.len(), "transition")?;
    let touched = keys::transition_keys(mdef, at);
    let old = mdef.transitions.remove(at);
    Ok(Effect::new(EditOp::AddTransition { machine: machine.to_owned(), transition: old, index: Some(at) }, touched))
}
