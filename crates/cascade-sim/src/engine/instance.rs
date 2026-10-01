//! A running machine instance: its fields, current leaf state and history.

use std::collections::HashMap;

use cascade_core::Model;
use cascade_core::ids::{MachineId, StateId, TransitionId};
use cascade_core::model::StateKind;

use crate::scenario::Payload;

/// Index into the engine's instance list, in creation order (declared or
/// added instances and spawned ones, as they were created). Removed
/// instances keep their index, so recorded steps stay valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct InstanceIx(pub usize);

#[derive(Clone, Debug)]
pub(crate) struct Instance {
    pub name: String,
    pub machine: MachineId,
    pub fields: Payload,
    /// Removed from play: no longer selectable or targetable. Its lifeline
    /// and last state stay in the trace.
    pub removed: bool,
    /// Always an atomic or final state of `machine`.
    leaf: StateId,
    /// The last active leaf under each compound state (`Some`) and under
    /// the machine's top level (`None`), for history pseudo-states.
    history: HashMap<Option<StateId>, StateId>,
}

impl Instance {
    /// A new instance in `start`, entered by default entry.
    pub fn new(model: &Model, name: String, machine: MachineId, fields: Payload, start: StateId) -> Self {
        Self { name, machine, fields, removed: false, leaf: model.default_entry(start), history: HashMap::new() }
    }

    pub fn leaf(&self) -> StateId {
        self.leaf
    }

    /// Take `transition` and return the new leaf. The leaf being left
    /// becomes the last active leaf under each of its ancestors, then the
    /// target is entered: a history target restores from that record, any
    /// other target is entered by default entry.
    pub fn take(&mut self, model: &Model, transition: TransitionId) -> StateId {
        let old = self.leaf;
        self.history.insert(None, old);
        for ancestor in model.ancestors(old) {
            self.history.insert(Some(ancestor), old);
        }
        self.leaf = self.enter(model, model.transition(transition).to);
        self.leaf
    }

    fn enter(&self, model: &Model, target: StateId) -> StateId {
        let state = model.state(target);
        let StateKind::History { deep } = state.kind else {
            return model.default_entry(target);
        };
        let parent = state.parent;
        match self.history.get(&parent) {
            // Never been inside the parent: enter it by default.
            None => model.default_entry(parent.unwrap_or_else(|| model.machine(self.machine).initial)),
            Some(&last) if deep => last,
            // Shallow: re-enter the parent's child that was active, by
            // default entry.
            Some(&last) => {
                let child =
                    std::iter::once(last).chain(model.ancestors(last)).find(|&s| model.state(s).parent == parent);
                model.default_entry(child.unwrap_or(last))
            }
        }
    }
}
