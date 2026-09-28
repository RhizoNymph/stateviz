//! Dispatch: apply one op to a definition in place and report its inverse
//! and touched keys. Nothing here validates the result; [`super::apply`]
//! resolves once at the end, which is what makes batches atomic.

use super::{EditError, EditOp, controllers, events, externals, lookup, machines, states, transitions};
use crate::definition::Definition;
use crate::key::ElementKey;

/// What an op did: how to undo it, and which elements it touched.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Effect {
    pub inverse: EditOp,
    pub touched: Vec<ElementKey>,
}

impl Effect {
    pub fn new(inverse: EditOp, touched: Vec<ElementKey>) -> Self {
        Self { inverse, touched }
    }
}

/// How much work to put into the inverse.
///
/// Some inverses are exact only after fix-ups (restoring the original
/// spelling of references a rename or insertion qualified), which are found
/// by simulating the primary inverse. Simulation itself only needs the
/// mutation, so it runs ops in `Primary` mode, which also keeps the
/// simulation from recursing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InverseMode {
    Exact,
    Primary,
}

pub(super) fn apply_op(def: &mut Definition, op: &EditOp, mode: InverseMode) -> Result<Effect, EditError> {
    match op {
        EditOp::AddMachine { machine, index } => machines::add(def, machine, *index),
        EditOp::RemoveMachine { machine } => machines::remove(def, machine),
        EditOp::RenameMachine { from, to } => machines::rename(def, from, to),
        EditOp::SetMachineColor { machine, color } => machines::set_color(def, machine, *color),
        EditOp::SetMachineDomain { machine, domain } => machines::set_domain(def, machine, domain.as_deref()),
        EditOp::SetMachineInitial { machine, initial } => machines::set_initial(def, machine, initial.as_deref()),
        EditOp::SetMachineFields { machine, fields } => machines::set_fields(def, machine, fields),

        EditOp::AddState { machine, parent, state, index } => {
            states::add(def, machine, parent.as_deref(), state, *index, mode)
        }
        EditOp::RemoveState { machine, path } => states::remove(def, machine, path, mode),
        EditOp::RenameState { machine, path, to } => states::rename(def, machine, path, to, mode),
        EditOp::SetStateKind { machine, path, kind } => states::set_kind(def, machine, path, *kind),
        EditOp::SetStateInitial { machine, path, initial } => {
            states::set_initial(def, machine, path, initial.as_deref())
        }

        EditOp::AddTransition { machine, transition, index } => transitions::add(def, machine, transition, *index),
        EditOp::UpdateTransition { machine, index, transition } => {
            transitions::update(def, machine, *index, transition)
        }
        EditOp::RemoveTransition { machine, index } => transitions::remove(def, machine, *index),

        EditOp::DeclareEvent { event, index } => events::declare(def, event, *index),
        EditOp::RemoveEventDeclaration { event } => events::remove_declaration(def, event, mode),
        EditOp::RenameEvent { from, to } => events::rename(def, from, to),

        EditOp::AddController { controller, index } => controllers::add(def, controller, *index),
        EditOp::RemoveController { controller } => controllers::remove(def, controller),
        EditOp::RenameController { from, to } => controllers::rename(def, from, to),
        EditOp::AddHandler { controller, handler, index } => controllers::add_handler(def, controller, handler, *index),
        EditOp::RemoveHandler { controller, event } => controllers::remove_handler(def, controller, event),
        EditOp::AddRule { controller, event, rule, index } => {
            controllers::add_rule(def, controller, event, rule, *index)
        }
        EditOp::UpdateRule { controller, event, index, rule } => {
            controllers::update_rule(def, controller, event, *index, rule)
        }
        EditOp::RemoveRule { controller, event, index } => controllers::remove_rule(def, controller, event, *index),

        EditOp::AddExternal { external, index } => externals::add(def, external, *index),
        EditOp::RemoveExternal { external } => externals::remove(def, external),
        EditOp::RenameExternal { from, to } => externals::rename(def, from, to),
        EditOp::SetExternalTriggers { external, triggers } => externals::set_triggers(def, external, triggers),

        EditOp::SetSystemName { name } => Ok(set_system_name(def, name.as_deref())),
        EditOp::Batch(ops) => batch(def, ops, mode),
    }
}

/// Apply `ops` in order; the inverse is the reversed inverses and the
/// touched keys are the union in first-touched order.
fn batch(def: &mut Definition, ops: &[EditOp], mode: InverseMode) -> Result<Effect, EditError> {
    let mut inverses = Vec::with_capacity(ops.len());
    let mut touched = Vec::new();
    for op in ops {
        let effect = apply_op(def, op, mode)?;
        inverses.push(effect.inverse);
        push_unique(&mut touched, effect.touched);
    }
    inverses.reverse();
    Ok(Effect::new(EditOp::Batch(inverses), touched))
}

fn set_system_name(def: &mut Definition, name: Option<&str>) -> Effect {
    let old = def.system.as_ref().map(|s| s.value.clone());
    lookup::set_optional(&mut def.system, name.map(str::to_owned));
    Effect::new(EditOp::SetSystemName { name: old }, Vec::new())
}

/// Apply `ops` to a copy of `def` without validating, to see what a
/// primary inverse leaves behind.
pub(super) fn simulate(def: &Definition, ops: &[EditOp]) -> Result<Definition, EditError> {
    let mut copy = def.clone();
    for op in ops {
        apply_op(&mut copy, op, InverseMode::Primary)?;
    }
    Ok(copy)
}

/// One op, or a batch when there are several.
pub(super) fn sequence(mut ops: Vec<EditOp>) -> EditOp {
    if ops.len() == 1
        && let Some(op) = ops.pop()
    {
        return op;
    }
    EditOp::Batch(ops)
}

pub(super) fn push_unique(into: &mut Vec<ElementKey>, keys: impl IntoIterator<Item = ElementKey>) {
    for key in keys {
        if !into.contains(&key) {
            into.push(key);
        }
    }
}
