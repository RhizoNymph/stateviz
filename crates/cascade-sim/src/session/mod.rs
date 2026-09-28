//! Interactive play: a simulator you drive one action at a time.
//!
//! A [`PlaySession`] holds instances, the FIFO queue and a [`Trace`] like a
//! batch run, but the host decides what happens next: add an instance, fire
//! an external trigger, deliver the queue's head or *any* pending item
//! (to explore orderings by hand), or run until the queue drains. Semantics
//! are exactly the batch simulator's (see the crate docs); the batch
//! `simulate` is a session driven by a scenario.
//!
//! Every action is recorded on a [`Timeline`]. Actions name things (machine,
//! instance, source and trigger names, not ids), so a session can be
//! [`PlaySession::replay`]ed against a new model after the definition is
//! edited, and saved as a scenario. Seeking back and then acting forks the
//! timeline: the abandoned future is kept as a [`Branch`] to return to.
//!
//! Owner: the `feat/sim-session` workstream implements everything here and
//! moves the batch simulator onto it. The types are the contract.

use std::collections::BTreeMap;

use cascade_core::Model;
use cascade_core::definition::TriggerRef;
use cascade_core::ids::{EventId, MachineId, RuleId, StateId};

use crate::error::SimError;
use crate::scenario::Scenario;
use crate::trace::{LifelineIx, StepIx, Trace};

/// One thing the player does. Model-independent: everything is by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlayAction {
    /// Create an instance. `name: None` picks `<machine-lowercase><n>`;
    /// `state: None` starts in the machine's initial state (a path
    /// otherwise).
    AddInstance {
        name: Option<String>,
        machine: String,
        fields: BTreeMap<String, String>,
        state: Option<String>,
    },
    RemoveInstance {
        name: String,
    },
    /// An external source fires a trigger at an instance. The trigger is
    /// delivered at once; the transition's emitted events join the queue.
    Fire {
        source: String,
        trigger: TriggerRef,
        target: String,
        payload: BTreeMap<String, String>,
    },
    /// Deliver one queue item: the head when `choice` is `None`, otherwise
    /// the pending item at that position in [`PlaySession::pending`].
    Step {
        choice: Option<u32>,
    },
    /// Deliver queue heads until the queue is empty (bounded by
    /// [`crate::STEP_LIMIT`]).
    RunUntilQuiet,
}

/// Stable within one session state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PendingId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingKind {
    /// An emitted event waiting to be delivered to its handlers.
    Event { event: EventId },
    /// A controller's fire waiting to be delivered to its target instance.
    Fire { rule: RuleId, target: LifelineIx },
}

/// A queue item, in queue order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingItem {
    pub id: PendingId,
    pub kind: PendingKind,
    /// The trace step that queued it.
    pub cause: StepIx,
    /// Short human label, e.g. `OrderPaid from o1` or `Fulfillment → s1: start`.
    pub label: String,
}

/// An instance's current situation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceState {
    pub lifeline: LifelineIx,
    pub name: String,
    pub machine: MachineId,
    /// The current leaf state.
    pub state: StateId,
    pub fields: BTreeMap<String, String>,
}

/// An external trigger the player could fire right now, for a palette of
/// buttons: every (source, trigger) pair × every instance of the trigger's
/// machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvailableFire {
    pub source: String,
    pub trigger: TriggerRef,
    pub target: String,
    /// Whether the target's current state accepts the trigger (firing a
    /// disabled one is allowed and records a drop).
    pub accepted: bool,
}

/// What one action produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionOutcome {
    /// Trace steps appended by the action.
    pub steps: std::ops::Range<usize>,
}

/// A saved alternative future.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    /// Where it forked from the timeline it was split from.
    pub fork: usize,
    pub actions: Vec<PlayAction>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Timeline {
    /// The current line of play.
    pub actions: Vec<PlayAction>,
    /// How many of `actions` are applied (seeking back lowers it).
    pub position: usize,
    pub branches: Vec<Branch>,
}

#[derive(Clone, Debug)]
pub struct PlaySession {
    timeline: Timeline,
    trace: Trace,
}

impl PlaySession {
    /// An empty session: no instances, empty queue.
    pub fn new(model: &Model) -> Self {
        let _ = model;
        Self {
            timeline: Timeline::default(),
            trace: Trace {
                scenario: String::new(),
                ordering: None,
                lifelines: Vec::new(),
                steps: Vec::new(),
                final_states: BTreeMap::new(),
            },
        }
    }

    /// Start from a scenario: its instances, then its steps as actions.
    ///
    /// Stub until `feat/sim-session` lands.
    pub fn from_scenario(model: &Model, scenario: &Scenario) -> Result<Self, SimError> {
        let _ = (model, scenario);
        Err(SimError::NotImplemented)
    }

    /// Perform an action and record it. When the timeline was seeked back,
    /// the old future is saved as a branch first.
    ///
    /// Stub until `feat/sim-session` lands.
    pub fn apply(&mut self, model: &Model, action: PlayAction) -> Result<ActionOutcome, SimError> {
        let _ = (model, action);
        Err(SimError::NotImplemented)
    }

    /// Move to `position` on the current timeline (0 = the start).
    ///
    /// Stub until `feat/sim-session` lands.
    pub fn seek(&mut self, model: &Model, position: usize) -> Result<(), SimError> {
        let _ = (model, position);
        Err(SimError::NotImplemented)
    }

    /// Make branch `index` the current timeline (the current future becomes
    /// a branch), positioned at its end.
    ///
    /// Stub until `feat/sim-session` lands.
    pub fn switch_branch(&mut self, model: &Model, index: usize) -> Result<(), SimError> {
        let _ = (model, index);
        Err(SimError::NotImplemented)
    }

    /// Rebuild this session against another model (after an edit), replaying
    /// the timeline up to its position. Stops at the first action that no
    /// longer applies and returns the session so far plus that error.
    ///
    /// Stub until `feat/sim-session` lands.
    pub fn replay(&self, model: &Model) -> (PlaySession, Option<(usize, SimError)>) {
        (PlaySession::new(model), Some((0, SimError::NotImplemented)))
    }

    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    pub fn trace(&self) -> &Trace {
        &self.trace
    }

    /// Instances in lifeline order with their current states.
    ///
    /// Stub until `feat/sim-session` lands.
    pub fn instances(&self) -> Vec<InstanceState> {
        Vec::new()
    }

    /// The queue, head first.
    ///
    /// Stub until `feat/sim-session` lands.
    pub fn pending(&self) -> Vec<PendingItem> {
        Vec::new()
    }

    /// Every external trigger × instance, with whether it would be accepted.
    ///
    /// Stub until `feat/sim-session` lands.
    pub fn available_fires(&self, model: &Model) -> Vec<AvailableFire> {
        let _ = model;
        Vec::new()
    }

    /// The timeline up to its position as a scenario. Manual queue choices
    /// (`Step { choice: Some(_) }`) are kept, so running the scenario
    /// reproduces this session's trace exactly.
    ///
    /// Stub until `feat/sim-session` lands.
    pub fn to_scenario(&self, name: &str) -> Result<Scenario, SimError> {
        let _ = name;
        Err(SimError::NotImplemented)
    }
}

/// Write a scenario as YAML in the scenario file format.
///
/// Stub until `feat/sim-session` lands.
pub fn scenario_to_yaml(scenario: &Scenario) -> String {
    let _ = scenario;
    String::new()
}
