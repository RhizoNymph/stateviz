//! Structural edits to a [`Definition`], for building systems in the app.
//!
//! Every edit is an [`EditOp`] addressed by names and stable positions, so it
//! stays meaningful across reloads. [`apply`] is pure: it returns the edited
//! definition, the inverse op (for undo), and the element keys it touched.
//! Renames propagate to every reference (transitions, initials, fires,
//! target selectors, external sources); removals cascade to what would
//! dangle (removing a state removes transitions touching it) and the inverse
//! restores all of it.
//!
//! The edited definition always resolves: an op that would leave dangling
//! references or invalid names is rejected with [`EditError`] and changes
//! nothing.
//!
//! Persisting an op to the YAML file without losing comments is
//! `cascade_interop::patch`, which applies the same op to the source text.
//!
//! Batches are atomic and validated once, at the end: intermediate steps may
//! pass through definitions that do not resolve (swapping two names through
//! a temporary, removing every event declaration of a strict file).
//!
//! The rules (what each op cascades to, its inverse, which names it
//! qualifies) are tabulated in `docs/features/build-and-play.md`, section
//! "Edit operations".

mod controllers;
mod engine;
mod events;
mod externals;
mod keys;
mod lookup;
mod machines;
mod refs;
mod restore;
mod spans;
mod states;
mod transitions;
mod validate;

pub use spans::without_spans;

use crate::color::PaletteColor;
use crate::definition::{
    ControllerDef, Definition, EventDef, ExternalDef, HandlerDef, MachineDef, RuleDef, StateDef, StateKindDef,
    TransitionDef, TriggerRef,
};
use crate::error::LoadError;
use crate::key::ElementKey;

/// Where to insert: `None` appends.
pub type Index = Option<usize>;

/// One structural edit. Machines, controllers, external sources and events
/// are addressed by name; states by dotted path within their machine;
/// transitions by their position in the machine's `transitions:` list;
/// handlers by (controller, event); rules by position within their handler.
#[derive(Clone, Debug, PartialEq)]
pub enum EditOp {
    // --- Machines -------------------------------------------------------
    AddMachine {
        machine: MachineDef,
        index: Index,
    },
    RemoveMachine {
        machine: String,
    },
    RenameMachine {
        from: String,
        to: String,
    },
    SetMachineColor {
        machine: String,
        color: Option<PaletteColor>,
    },
    SetMachineDomain {
        machine: String,
        domain: Option<String>,
    },
    /// `None` falls back to the first state.
    SetMachineInitial {
        machine: String,
        initial: Option<String>,
    },
    SetMachineFields {
        machine: String,
        fields: Vec<String>,
    },

    // --- States -----------------------------------------------------------
    /// `parent` is a state path; `None` adds a top-level state.
    AddState {
        machine: String,
        parent: Option<String>,
        state: StateDef,
        index: Index,
    },
    /// Also removes transitions from or to the state or its descendants.
    RemoveState {
        machine: String,
        path: String,
    },
    RenameState {
        machine: String,
        path: String,
        to: String,
    },
    SetStateKind {
        machine: String,
        path: String,
        kind: StateKindDef,
    },
    /// Initial child of a compound state; `None` falls back to the first.
    SetStateInitial {
        machine: String,
        path: String,
        initial: Option<String>,
    },

    // --- Transitions --------------------------------------------------------
    AddTransition {
        machine: String,
        transition: TransitionDef,
        index: Index,
    },
    /// Replace the whole entry (from, to, trigger, guard, emits, bounded).
    UpdateTransition {
        machine: String,
        index: usize,
        transition: TransitionDef,
    },
    RemoveTransition {
        machine: String,
        index: usize,
    },

    // --- Events -------------------------------------------------------------
    /// Add to `events:` (switching the file to strict event declarations if
    /// it had none: every emitted or subscribed event is then declared).
    DeclareEvent {
        event: EventDef,
        index: Index,
    },
    RemoveEventDeclaration {
        event: String,
    },
    /// Renames the event everywhere: declarations, emits, subscriptions.
    RenameEvent {
        from: String,
        to: String,
    },

    // --- Controllers ----------------------------------------------------------
    AddController {
        controller: ControllerDef,
        index: Index,
    },
    RemoveController {
        controller: String,
    },
    RenameController {
        from: String,
        to: String,
    },
    AddHandler {
        controller: String,
        handler: HandlerDef,
        index: Index,
    },
    RemoveHandler {
        controller: String,
        event: String,
    },
    AddRule {
        controller: String,
        event: String,
        rule: RuleDef,
        index: Index,
    },
    UpdateRule {
        controller: String,
        event: String,
        index: usize,
        rule: RuleDef,
    },
    RemoveRule {
        controller: String,
        event: String,
        index: usize,
    },

    // --- External sources -------------------------------------------------------
    AddExternal {
        external: ExternalDef,
        index: Index,
    },
    RemoveExternal {
        external: String,
    },
    RenameExternal {
        from: String,
        to: String,
    },
    SetExternalTriggers {
        external: String,
        triggers: Vec<TriggerRef>,
    },

    // --- Other ------------------------------------------------------------------
    SetSystemName {
        name: Option<String>,
    },
    /// Apply in order, atomically; the inverse is the reversed inverses.
    Batch(Vec<EditOp>),
}

/// The result of a successful edit.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
    pub definition: Definition,
    /// Applying this to `definition` restores the original.
    pub inverse: EditOp,
    /// Stable keys of elements added, changed or removed, for selection and
    /// highlighting after the edit. Keys are as of the edited definition for
    /// additions and changes, and as of the original for removals.
    pub touched: Vec<ElementKey>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("no {what} `{name}`")]
    NotFound { what: &'static str, name: String },
    #[error("{what} `{name}` already exists")]
    NameTaken { what: &'static str, name: String },
    #[error("`{0}` is not a valid name")]
    InvalidName(String),
    #[error("index {index} is out of range for {what} (length {len})")]
    IndexOutOfRange { what: &'static str, index: usize, len: usize },
    /// The op is well-formed but the result would not resolve.
    #[error("the edit would make the definition invalid:\n{0}")]
    Invalid(LoadError),
    #[error("editing is not implemented yet")]
    NotImplemented,
}

/// Apply one op to a definition.
///
/// Pure: `definition` is untouched. The result always resolves; an op that
/// would leave the definition unresolvable is rejected with
/// [`EditError::Invalid`], and an op naming something that does not exist,
/// reusing a taken name, using an invalid name or an out-of-range index is
/// rejected with the matching error.
pub fn apply(definition: &Definition, op: &EditOp) -> Result<Applied, EditError> {
    let mut edited = definition.clone();
    let effect = engine::apply_op(&mut edited, op, engine::InverseMode::Exact)?;
    crate::resolve::resolve(edited.clone()).map_err(EditError::Invalid)?;
    Ok(Applied { definition: edited, inverse: effect.inverse, touched: effect.touched })
}

/// Find the `transitions:` entry of a transition key: `(machine name, index
/// in that machine's list)`. A multi-source entry (`from: [a, b]`) is found
/// from the key of any of its expansions.
pub fn locate_transition(definition: &Definition, key: &ElementKey) -> Option<(String, usize)> {
    keys::locate_transition(definition, key)
}

/// A fresh name based on `base` that is not in `taken`: `base`, `base2`,
/// `base3`, … Used for "add machine/state/controller" with default names.
pub fn fresh_name<'a>(base: &str, taken: impl IntoIterator<Item = &'a str>) -> String {
    let taken: std::collections::HashSet<&str> = taken.into_iter().collect();
    if !taken.contains(base) {
        return base.to_owned();
    }
    (2u32..)
        .map(|n| format!("{base}{n}"))
        .find(|candidate| !taken.contains(candidate.as_str()))
        .unwrap_or_else(|| base.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_names_skip_taken_ones() {
        assert_eq!(fresh_name("State", ["Other"]), "State");
        assert_eq!(fresh_name("State", ["State", "State2"]), "State3");
    }
}
