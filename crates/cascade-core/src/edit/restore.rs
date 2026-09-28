//! Exact inverses for state edits.
//!
//! A state edit's primary inverse (rename back, remove the added state,
//! re-add the removed one) restores the tree, but references the edit
//! qualified to keep them unambiguous stay qualified, and initials it reset
//! stay reset. Here the primary inverse is simulated and completed with
//! fix-ups that restore every transition entry, the machine initial and
//! every compound initial exactly as they were.

use super::engine::{self, InverseMode};
use super::refs::{StateIndex, find};
use super::{EditError, EditOp, spans};
use crate::definition::{Definition, MachineDef};

/// `primary`, completed so that applying it to `def` restores machine
/// `machine` to `original`. In [`InverseMode::Primary`] the fix-ups are
/// skipped.
pub(super) fn machine_inverse(
    def: &Definition,
    machine: usize,
    original: &MachineDef,
    primary: Vec<EditOp>,
    mode: InverseMode,
) -> Result<EditOp, EditError> {
    if mode == InverseMode::Primary {
        return Ok(engine::sequence(primary));
    }
    let simulated = engine::simulate(def, &primary)?;
    let mut ops = primary;
    if let Some(current) = simulated.machines.get(machine) {
        ops.extend(fixups(original, current));
    }
    Ok(engine::sequence(ops))
}

/// Ops that turn `current`'s transitions and initials into `original`'s,
/// given that both have the same state tree.
fn fixups(original: &MachineDef, current: &MachineDef) -> Vec<EditOp> {
    let machine = original.name.value.clone();
    let mut ops = Vec::new();

    let (want, have) = (&original.transitions, &current.transitions);
    for (i, (w, h)) in want.iter().zip(have.iter()).enumerate() {
        if spans::transition(w) != spans::transition(h) {
            ops.push(EditOp::UpdateTransition { machine: machine.clone(), index: i, transition: w.clone() });
        }
    }
    for i in (want.len()..have.len()).rev() {
        ops.push(EditOp::RemoveTransition { machine: machine.clone(), index: i });
    }
    for (i, w) in want.iter().enumerate().skip(have.len()) {
        ops.push(EditOp::AddTransition { machine: machine.clone(), transition: w.clone(), index: Some(i) });
    }

    let value = |initial: &Option<crate::span::Spanned<String>>| initial.as_ref().map(|i| i.value.clone());
    if value(&original.initial) != value(&current.initial) {
        ops.push(EditOp::SetMachineInitial { machine: machine.clone(), initial: value(&original.initial) });
    }

    for path in StateIndex::of(&original.states).paths() {
        let (Some(want), Some(have)) = (find(&original.states, path), find(&current.states, path)) else {
            continue;
        };
        if value(&want.initial) != value(&have.initial) {
            ops.push(EditOp::SetStateInitial {
                machine: machine.clone(),
                path: path.to_owned(),
                initial: value(&want.initial),
            });
        }
    }
    ops
}
