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

/// What taking a transition did to an instance's configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Change {
    /// The new leaf.
    pub to: StateId,
    /// States left, innermost first.
    pub exited: Vec<StateId>,
    /// States entered, outermost first.
    pub entered: Vec<StateId>,
}

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

    /// The history record sorted by key, for canonical state keys.
    pub fn history_entries(&self) -> Vec<(Option<StateId>, StateId)> {
        let mut entries: Vec<_> = self.history.iter().map(|(&k, &v)| (k, v)).collect();
        entries.sort();
        entries
    }

    /// Every state the instance is in, outermost first (the leaf last): the
    /// states entered when it is created.
    pub fn active_states(&self, model: &Model) -> Vec<StateId> {
        let mut states: Vec<StateId> = std::iter::once(self.leaf).chain(model.ancestors(self.leaf)).collect();
        states.reverse();
        states
    }

    /// Take `transition`. The leaf being left becomes the last active leaf
    /// under each of its ancestors, then the target is entered: a history
    /// target restores from that record, any other target is entered by
    /// default entry.
    ///
    /// The states left and entered are those below the transition's
    /// *domain*, the nearest state that properly contains both its source
    /// and its target (the machine's top level when none does), so a self
    /// transition leaves and re-enters its state.
    pub fn take(&mut self, model: &Model, transition: TransitionId) -> Change {
        let t = model.transition(transition);
        let domain = model.ancestors(t.from).find(|&a| model.ancestors(t.to).any(|b| b == a));
        let below = |leaf: StateId| -> Vec<StateId> {
            std::iter::once(leaf).chain(model.ancestors(leaf)).take_while(|&s| Some(s) != domain).collect()
        };

        let old = self.leaf;
        self.history.insert(None, old);
        for ancestor in model.ancestors(old) {
            self.history.insert(Some(ancestor), old);
        }
        self.leaf = self.enter(model, t.to);
        let mut entered = below(self.leaf);
        entered.reverse();
        Change { to: self.leaf, exited: below(old), entered }
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
